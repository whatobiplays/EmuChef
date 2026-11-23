#define _GNU_SOURCE

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#ifdef O_PATH
#define DIRECTORY_OPEN_FLAGS O_PATH
#else
#define DIRECTORY_OPEN_FLAGS O_RDONLY
#endif

struct path_parts {
    char *storage;
    char **items;
    size_t count;
};

struct operation_status {
    int active;
    int stage_fd;
    int mutation_started;
    int complete;
    unsigned int update_sequence;
    char operation_id[129];
    char action[16];
    char source_sha256[65];
    uint64_t source_length;
    char original_sha256[65];
    uint64_t original_length;
    char phase[16];
    char result_sha256[65];
    uint64_t result_length;
    char write_state[16];
    char flush_state[16];
    char status_name[160];
};

static struct operation_status current_operation_status = { .stage_fd = -1 };

static void fail(const char *action) {
    fprintf(stderr, "selected-root helper %s: %s\n", action, strerror(errno));
    exit(1);
}
static void fail_with_status(int status, const char *action) {
    fprintf(stderr, "selected-root helper %s: %s\n", action, strerror(errno));
    exit(status);
}

static void sync_directory(int directory_fd, const char *action) {
    int readable = openat(directory_fd, ".", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (readable < 0) {
        fail("open selected-root directory for flush");
    }
    if (fsync(readable) < 0) {
        fail(action);
    }
    close(readable);
}
static void test_final_open_race(int parent_fd, const char *name) {
#ifdef EMUCHEF_COPY_COMMIT_HELPER_TEST
    const char *race = getenv("EMU_CHEF_TEST_FINAL_OPEN_RACE");
    if (race != NULL && strcmp(race, "file") == 0) {
        int fd = openat(parent_fd, name,
                        O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                        0644);
        if (fd >= 0) {
            const char marker[] = "concurrent target";
            (void)write(fd, marker, sizeof(marker) - 1);
            (void)fsync(fd);
            close(fd);
        }
    } else if (race != NULL && strcmp(race, "symlink") == 0) {
        (void)symlinkat("/emuchef-test-victim", parent_fd, name);
    }
#else
    (void)parent_fd;
    (void)name;
#endif
}

static void test_partial_final_write(int destination_fd) {
#ifdef EMUCHEF_COPY_COMMIT_HELPER_TEST
    const char *partial = getenv("EMU_CHEF_TEST_PARTIAL_FINAL_WRITE");
    if (partial != NULL && strcmp(partial, "1") == 0) {
        const char marker[] = "partial";
        if (write(destination_fd, marker, sizeof(marker) - 1) < 0 ||
            fsync(destination_fd) < 0) {
            fail_with_status(12, "test injected partial configuration write");
        }
        errno = EIO;
        fail_with_status(12, "test injected partial configuration write");
    }
#else
    (void)destination_fd;
#endif
}

/* A raced FIFO must not block final verification. */
static int open_final_configuration_readback(int parent_fd, const char *name,
                                             int flags) {
#ifdef EMUCHEF_COPY_COMMIT_HELPER_TEST
    const char *race = getenv("EMU_CHEF_TEST_FINAL_READBACK_RACE");
    if (race != NULL && strcmp(race, "fifo") == 0) {
        if (unlinkat(parent_fd, name, 0) < 0 ||
            mkfifoat(parent_fd, name, 0600) < 0) {
            return -1;
        }
        if ((flags & O_NONBLOCK) == 0) {
            errno = EDEADLK;
            return -1;
        }
    }
#endif
    return openat(parent_fd, name, flags);
}

static bool safe_component(const char *part) {
    return part[0] != '\0' && strcmp(part, ".") != 0 && strcmp(part, "..") != 0;
}

static struct path_parts split_relative(const char *path) {
    struct path_parts result = {0};
    size_t length = strlen(path);
    if (length == 0 || path[0] == '/' || path[length - 1] == '/' || strstr(path, "//")) {
        errno = EINVAL;
        fail("relative path validation");
    }

    result.storage = strdup(path);
    result.items = calloc(length / 2 + 2, sizeof(char *));
    if (result.storage == NULL || result.items == NULL) {
        fail("allocate path");
    }

    char *save = NULL;
    for (char *part = strtok_r(result.storage, "/", &save); part != NULL;
         part = strtok_r(NULL, "/", &save)) {
        if (!safe_component(part)) {
            errno = EINVAL;
            fail("relative path validation");
        }
        result.items[result.count++] = part;
    }
    if (result.count == 0) {
        errno = EINVAL;
        fail("relative path validation");
    }
    return result;
}

static int open_absolute_directory(const char *path) {
    if (path[0] != '/' || path[1] == '\0' || strstr(path, "//") != NULL) {
        errno = EINVAL;
        fail("absolute directory validation");
    }

    char *storage = strdup(path + 1);
    if (storage == NULL) {
        fail("allocate absolute path");
    }
    int current = open("/", DIRECTORY_OPEN_FLAGS | O_DIRECTORY | O_CLOEXEC);
    if (current < 0) {
        fail("open filesystem root");
    }

    char *save = NULL;
    for (char *part = strtok_r(storage, "/", &save); part != NULL;
         part = strtok_r(NULL, "/", &save)) {
        if (!safe_component(part)) {
            errno = EINVAL;
            fail("absolute path validation");
        }
        int next = openat(current, part,
                          DIRECTORY_OPEN_FLAGS | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
        if (next < 0) {
            fail("open no-follow directory component");
        }
        close(current);
        current = next;
    }
    free(storage);
    return current;
}

static int open_relative_directory(int root_fd, const struct path_parts *path,
                                   size_t count, bool create) {
    int current = dup(root_fd);
    if (current < 0) {
        fail("duplicate selected-root descriptor");
    }

    for (size_t index = 0; index < count; index++) {
        if (create && mkdirat(current, path->items[index], 0755) < 0 && errno != EEXIST) {
            fail("create selected-root directory");
        }
        int next = openat(current, path->items[index],
                          DIRECTORY_OPEN_FLAGS | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
        if (next < 0) {
            fail("open no-follow selected-root directory");
        }
        close(current);
        current = next;
    }
    return current;
}

static void require_link_target(int parent_fd, const char *name, const char *expected) {
    char target[256];
    ssize_t length = readlinkat(parent_fd, name, target, sizeof(target) - 1);
    if (length < 0) {
        fail("read reviewed shared-storage alias");
    }
    if ((size_t)length >= sizeof(target) - 1) {
        errno = ENAMETOOLONG;
        fail("reviewed shared-storage alias validation");
    }
    target[length] = '\0';
    if (strcmp(target, expected) != 0) {
        errno = ESTALE;
        fail("reviewed shared-storage alias changed");
    }
}

static void verify_shared_storage_alias_chain(void) {
    int root = open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (root < 0) {
        fail("open filesystem root for alias validation");
    }
    require_link_target(root, "sdcard", "/storage/self/primary");
    close(root);

    int self = open_absolute_directory("/storage/self");
    require_link_target(self, "primary", "/storage/emulated/0");
    close(self);

    int shared_root = open_absolute_directory("/storage/emulated/0");
    close(shared_root);
}

static void verify_binding_canonical_path(const char *canonical_path, const char *expected) {
    if (strcmp(canonical_path, expected) != 0) {
        errno = ESTALE;
        fail("reviewed canonical destination changed");
    }
}

/* Resolve only the reviewed Android shared-storage aliases; descendants stay no-follow. */
static int open_reviewed_selected_root(const char *path, const char *canonical_path) {
    static const char sdcard_prefix[] = "/sdcard/";
    static const char primary_prefix[] = "/storage/self/primary/";
    static const char emulated_prefix[] = "/storage/emulated/0/";
    static const char canonical_base[] = "/storage/emulated/0";
    const char *suffix = NULL;

    if (strncmp(path, sdcard_prefix, sizeof(sdcard_prefix) - 1) == 0) {
        suffix = path + sizeof(sdcard_prefix) - 1;
    } else if (strncmp(path, primary_prefix, sizeof(primary_prefix) - 1) == 0) {
        suffix = path + sizeof(primary_prefix) - 1;
    } else if (strcmp(path, "/sdcard") == 0 ||
               strcmp(path, "/storage/self/primary") == 0) {
        suffix = "";
    } else {
        verify_binding_canonical_path(canonical_path, path);
        if (strcmp(path, canonical_base) == 0 ||
            strncmp(path, emulated_prefix, sizeof(emulated_prefix) - 1) == 0) {
            verify_shared_storage_alias_chain();
        }
        return open_absolute_directory(path);
    }

    verify_shared_storage_alias_chain();
    char expected[4096];
    int length = suffix[0] == '\0'
                     ? snprintf(expected, sizeof(expected), "%s", canonical_base)
                     : snprintf(expected, sizeof(expected), "%s/%s", canonical_base, suffix);
    if (length < 0 || (size_t)length >= sizeof(expected)) {
        errno = ENAMETOOLONG;
        fail("reviewed canonical destination validation");
    }
    verify_binding_canonical_path(canonical_path, expected);

    int shared_root = open_absolute_directory(canonical_base);
    if (suffix[0] == '\0') {
        return shared_root;
    }
    struct path_parts relative = split_relative(suffix);
    int selected = open_relative_directory(shared_root, &relative, relative.count, false);
    close(shared_root);
    free(relative.items);
    free(relative.storage);
    return selected;
}

static int open_relative_parent(int root_fd, struct path_parts *path) {
    if (path->count == 1) {
        int parent = dup(root_fd);
        if (parent < 0) {
            fail("duplicate selected-root descriptor");
        }
        return parent;
    }
    return open_relative_directory(root_fd, path, path->count - 1, false);
}

static void verify_selected_identity(int root_fd, const char *selected) {
    char *separator = strchr(selected, ':');
    if (separator == NULL) {
        errno = EINVAL;
        fail("reviewed destination identity validation");
    }

    char device_text[32];
    size_t device_length = (size_t)(separator - selected);
    if (device_length == 0 || device_length >= sizeof(device_text)) {
        errno = EINVAL;
        fail("reviewed destination device validation");
    }
    memcpy(device_text, selected, device_length);
    device_text[device_length] = '\0';
    char *device_end = NULL;
    char *inode_end = NULL;
    uintmax_t expected_device = strtoumax(device_text, &device_end, 10);
    uintmax_t expected_inode = strtoumax(separator + 1, &inode_end, 10);
    if (device_end == device_text || *device_end != '\0' || inode_end == separator + 1 ||
        *inode_end != '\0') {
        errno = EINVAL;
        fail("reviewed destination identity parsing");
    }

    struct stat actual;
    if (fstat(root_fd, &actual) < 0) {
        fail("inspect selected-root descriptor");
    }
    if ((uintmax_t)actual.st_dev != expected_device ||
        (uintmax_t)actual.st_ino != expected_inode) {
        errno = ESTALE;
        fail("selected-root identity changed after review");
    }
}

static void stale_entry(void) {
    errno = ESTALE;
    fail("reviewed destination entry changed after review");
}

static void verify_entry_identity(const struct stat *status, const char *identity) {
    const char *separator = strchr(identity, ':');
    if (separator == NULL || strchr(separator + 1, ':') != NULL) {
        errno = EINVAL;
        fail("reviewed destination entry identity validation");
    }
    char device_text[32];
    size_t device_length = (size_t)(separator - identity);
    if (device_length == 0 || device_length >= sizeof(device_text)) {
        errno = EINVAL;
        fail("reviewed destination entry device validation");
    }
    memcpy(device_text, identity, device_length);
    device_text[device_length] = '\0';
    char *device_end = NULL;
    char *inode_end = NULL;
    uintmax_t expected_device = strtoumax(device_text, &device_end, 10);
    uintmax_t expected_inode = strtoumax(separator + 1, &inode_end, 10);
    if (device_end == device_text || *device_end != '\0' || inode_end == separator + 1 ||
        *inode_end != '\0') {
        errno = EINVAL;
        fail("reviewed destination entry identity parsing");
    }
    if ((uintmax_t)status->st_dev != expected_device ||
        (uintmax_t)status->st_ino != expected_inode) {
        stale_entry();
    }
}

static void sha256_from_fd(int file_fd, char output[65]) {
    int descriptors[2];
    if (pipe(descriptors) < 0) {
        fail("create reviewed-entry digest pipe");
    }
    pid_t child = fork();
    if (child < 0) {
        close(descriptors[0]);
        close(descriptors[1]);
        fail("start reviewed-entry digest process");
    }
    if (child == 0) {
        close(descriptors[0]);
        if (dup2(file_fd, STDIN_FILENO) < 0 ||
            dup2(descriptors[1], STDOUT_FILENO) < 0) {
            _exit(126);
        }
        close(descriptors[1]);
        execlp("sha256sum", "sha256sum", (char *)NULL);
        _exit(127);
    }

    close(descriptors[1]);
    char digest_output[128];
    size_t used = 0;
    while (used < sizeof(digest_output) - 1) {
        ssize_t count = read(descriptors[0], digest_output + used,
                             sizeof(digest_output) - 1 - used);
        if (count == 0) {
            break;
        }
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            close(descriptors[0]);
            fail("read reviewed-entry digest");
        }
        used += (size_t)count;
    }
    close(descriptors[0]);
    digest_output[used] = '\0';

    int status = 0;
    while (waitpid(child, &status, 0) < 0) {
        if (errno != EINTR) {
            fail("wait for reviewed-entry digest");
        }
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0 || used < 64) {
        errno = EIO;
        fail("compute reviewed-entry digest");
    }
    memcpy(output, digest_output, 64);
    output[64] = '\0';
}

static void verify_entry_precondition(int parent_fd, const char *name,
                                      const char *expected) {
    if (strcmp(expected, "absent") == 0) {
        struct stat status;
        if (fstatat(parent_fd, name, &status, AT_SYMLINK_NOFOLLOW) == 0) {
            stale_entry();
        }
        if (errno != ENOENT) {
            fail("inspect reviewed destination entry");
        }
        return;
    }

    struct stat status;
    if (fstatat(parent_fd, name, &status, AT_SYMLINK_NOFOLLOW) < 0) {
        if (errno == ENOENT) {
            stale_entry();
        }
        fail("inspect reviewed destination entry");
    }

    if (strncmp(expected, "directory|", 10) == 0) {
        if (!S_ISDIR(status.st_mode)) {
            stale_entry();
        }
        verify_entry_identity(&status, expected + 10);
        return;
    }

    if (strncmp(expected, "file|", 5) != 0) {
        errno = EINVAL;
        fail("reviewed destination entry precondition validation");
    }
    const char *identity = expected + 5;
    const char *separator = strchr(identity, '|');
    if (separator == NULL || !S_ISREG(status.st_mode)) {
        stale_entry();
    }
    size_t identity_length = (size_t)(separator - identity);
    if (identity_length == 0 || identity_length >= 96) {
        errno = EINVAL;
        fail("reviewed destination file identity validation");
    }
    char identity_text[96];
    memcpy(identity_text, identity, identity_length);
    identity_text[identity_length] = '\0';
    verify_entry_identity(&status, identity_text);

    const char *expected_digest = separator + 1;
    if (strlen(expected_digest) != 64) {
        errno = EINVAL;
        fail("reviewed destination file digest validation");
    }
    int file_fd = openat(parent_fd, name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (file_fd < 0) {
        if (errno == ENOENT || errno == ELOOP) {
            stale_entry();
        }
        fail("open reviewed destination file");
    }
    struct stat opened_status;
    if (fstat(file_fd, &opened_status) < 0) {
        close(file_fd);
        fail("inspect reviewed destination file");
    }
    if (opened_status.st_dev != status.st_dev || opened_status.st_ino != status.st_ino) {
        close(file_fd);
        stale_entry();
    }
    char actual_digest[65];
    sha256_from_fd(file_fd, actual_digest);
    close(file_fd);
    if (strcmp(actual_digest, expected_digest) != 0) {
        stale_entry();
    }
}

static void ensure_directory(int root_fd, const char *relative, const char *expected) {
    struct path_parts path = split_relative(relative);
    int parent = open_relative_parent(root_fd, &path);
    if (strcmp(expected, "absent") == 0) {
        verify_entry_precondition(parent, path.items[path.count - 1], expected);
        if (mkdirat(parent, path.items[path.count - 1], 0755) < 0) {
            fail("create selected-root directory");
        }
        sync_directory(parent, "flush selected-root parent after directory creation");
    } else if (strncmp(expected, "directory|", 10) == 0) {
        verify_entry_precondition(parent, path.items[path.count - 1], expected);
    } else {
        errno = EINVAL;
        fail("selected-root directory precondition validation");
    }
    int directory = openat(parent, path.items[path.count - 1],
                           DIRECTORY_OPEN_FLAGS | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    if (directory < 0) {
        fail("open selected-root directory after validation");
    }
    close(directory);
    close(parent);
    free(path.items);
    free(path.storage);
}

static void remove_entry(int root_fd, const char *relative, bool directory,
                         const char *expected) {
    struct path_parts path = split_relative(relative);
    int parent = open_relative_parent(root_fd, &path);
    struct stat status;
    if (fstatat(parent, path.items[path.count - 1], &status, AT_SYMLINK_NOFOLLOW) < 0) {
        fail("inspect selected-root entry");
    }
    if (directory ? !S_ISDIR(status.st_mode) : !S_ISREG(status.st_mode)) {
        errno = EINVAL;
        fail("selected-root entry type changed after review");
    }
    verify_entry_precondition(parent, path.items[path.count - 1], expected);
    if (unlinkat(parent, path.items[path.count - 1], directory ? AT_REMOVEDIR : 0) < 0) {
        fail("remove selected-root entry");
    }
    sync_directory(parent, "flush selected-root directory after removal");
    close(parent);
    free(path.items);
    free(path.storage);
}

static int valid_operation_id(const char *operation_id);
static void await_operation_commit(const char *operation_id);

static int write_all_status(int descriptor, const char *bytes, size_t length) {
    size_t offset = 0;
    while (offset < length) {
        ssize_t count = write(descriptor, bytes + offset, length - offset);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count <= 0) {
            return -1;
        }
        offset += (size_t)count;
    }
    return 0;
}

static int flush_status_directory(int directory_fd) {
    int readable = openat(directory_fd, ".", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (readable < 0) {
        return -1;
    }
    int result = fsync(readable);
    int saved_errno = errno;
    close(readable);
    errno = saved_errno;
    return result;
}

static int write_operation_status(int initial) {
    struct operation_status *status = &current_operation_status;
    char line[512];
    int length = snprintf(
        line, sizeof(line),
        "STATUS 1 %s %s %s retroarch.cfg %s %" PRIu64 " %s %" PRIu64
        " %s %" PRIu64 " %s %s\n",
        status->operation_id, status->action, status->phase,
        status->source_sha256, status->source_length,
        status->original_sha256, status->original_length,
        status->result_sha256, status->result_length,
        status->write_state, status->flush_state);
    if (length <= 0 || (size_t)length >= sizeof(line)) {
        errno = EOVERFLOW;
        return -1;
    }

    if (initial) {
        int descriptor = openat(status->stage_fd, status->status_name,
                                O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                                0600);
        if (descriptor < 0) {
            return -1;
        }
        int result = write_all_status(descriptor, line, (size_t)length);
        if (result == 0) {
            result = fsync(descriptor);
        }
        int saved_errno = errno;
        close(descriptor);
        errno = saved_errno;
        if (result < 0 || flush_status_directory(status->stage_fd) < 0) {
            return -1;
        }
        return 0;
    }

    char temporary[192];
    int temporary_length = snprintf(
        temporary, sizeof(temporary), ".%s.%ld.%u.tmp",
        status->status_name, (long)getpid(), status->update_sequence++);
    if (temporary_length <= 0 || (size_t)temporary_length >= sizeof(temporary)) {
        errno = EOVERFLOW;
        return -1;
    }
    int descriptor = openat(status->stage_fd, temporary,
                            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                            0600);
    if (descriptor < 0) {
        return -1;
    }
    int result = write_all_status(descriptor, line, (size_t)length);
    if (result == 0) {
        result = fsync(descriptor);
    }
    int saved_errno = errno;
    close(descriptor);
    errno = saved_errno;
    if (result < 0 || renameat(status->stage_fd, temporary,
                               status->stage_fd, status->status_name) < 0) {
        saved_errno = errno;
        unlinkat(status->stage_fd, temporary, 0);
        errno = saved_errno;
        return -1;
    }
    return flush_status_directory(status->stage_fd);
}

static void finalize_operation_status(void) {
    struct operation_status *status = &current_operation_status;
    if (!status->active) {
        return;
    }
    if (strcmp(status->phase, "complete") != 0 &&
        strcmp(status->phase, "aborted") != 0) {
        if (status->mutation_started) {
            snprintf(status->phase, sizeof(status->phase), "failed");
            snprintf(status->result_sha256, sizeof(status->result_sha256), "unknown");
            status->result_length = 0;
            snprintf(status->write_state, sizeof(status->write_state), "unknown");
            snprintf(status->flush_state, sizeof(status->flush_state), "unknown");
        } else {
            snprintf(status->phase, sizeof(status->phase), "aborted");
        }
        (void)write_operation_status(0);
    }
    close(status->stage_fd);
    status->stage_fd = -1;
    status->active = 0;
}

static void begin_operation_status(const char *stage_root, const char *operation_id,
                                  const char *action, const char *source_sha256,
                                  uint64_t source_length, const char *original_sha256,
                                  uint64_t original_length) {
    struct operation_status *status = &current_operation_status;
    if (!valid_operation_id(operation_id) ||
        (strcmp(action, "create") != 0 && strcmp(action, "replace") != 0) ||
        strlen(source_sha256) != 64 || strlen(original_sha256) > 64) {
        errno = EINVAL;
        fail("invalid operation status identity");
    }
    status->stage_fd = open_absolute_directory(stage_root);
    snprintf(status->operation_id, sizeof(status->operation_id), "%s", operation_id);
    snprintf(status->action, sizeof(status->action), "%s", action);
    snprintf(status->source_sha256, sizeof(status->source_sha256), "%s", source_sha256);
    status->source_length = source_length;
    snprintf(status->original_sha256, sizeof(status->original_sha256), "%s", original_sha256);
    status->original_length = original_length;
    snprintf(status->phase, sizeof(status->phase), "ready");
    snprintf(status->result_sha256, sizeof(status->result_sha256), "unknown");
    status->result_length = 0;
    snprintf(status->write_state, sizeof(status->write_state), "none");
    snprintf(status->flush_state, sizeof(status->flush_state), "none");
    int name_length = snprintf(status->status_name, sizeof(status->status_name),
                               ".operation-%s.v1", operation_id);
    if (name_length <= 0 || (size_t)name_length >= sizeof(status->status_name) ||
        write_operation_status(1) < 0) {
        fail("persist initial operation status");
    }
    status->active = 1;
    if (atexit(finalize_operation_status) != 0) {
        status->phase[0] = '\0';
        (void)write_operation_status(0);
        status->active = 0;
        close(status->stage_fd);
        status->stage_fd = -1;
        fail("register operation status finalizer");
    }
}

static void mark_operation_commit_pending(void) {
    struct operation_status *status = &current_operation_status;
    snprintf(status->phase, sizeof(status->phase), "commit_pending");
    if (write_operation_status(0) < 0) {
        fail("persist operation COMMIT status");
    }
}

static void mark_operation_mutation_started(void) {
    current_operation_status.mutation_started = 1;
}

static void complete_operation_status(const char *result_sha256,
                                      uint64_t result_length) {
    struct operation_status *status = &current_operation_status;
    status->complete = 1;
    snprintf(status->phase, sizeof(status->phase), "complete");
    snprintf(status->result_sha256, sizeof(status->result_sha256), "%s", result_sha256);
    status->result_length = result_length;
    snprintf(status->write_state, sizeof(status->write_state), "complete");
    snprintf(status->flush_state, sizeof(status->flush_state), "complete");
    if (write_operation_status(0) < 0) {
        fail("persist completed operation status");
    }
}

static void query_operation_status(const char *stage_root, const char *operation_id) {
    if (!valid_operation_id(operation_id)) {
        errno = EINVAL;
        fail("invalid operation status query identity");
    }
    int stage_fd = open_absolute_directory(stage_root);
    char name[160];
    char prefix[160];
    int name_length = snprintf(name, sizeof(name), ".operation-%s.v1", operation_id);
    int prefix_length = snprintf(prefix, sizeof(prefix), "STATUS 1 %s ", operation_id);
    if (name_length <= 0 || (size_t)name_length >= sizeof(name) ||
        prefix_length <= 0 || (size_t)prefix_length >= sizeof(prefix)) {
        close(stage_fd);
        errno = EOVERFLOW;
        fail("format operation status query");
    }
    int descriptor = openat(stage_fd, name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (descriptor < 0) {
        close(stage_fd);
        fail("open durable operation status");
    }
    struct stat status;
    if (fstat(descriptor, &status) < 0 || !S_ISREG(status.st_mode) ||
        status.st_size <= 0 || status.st_size > 1024) {
        close(descriptor);
        close(stage_fd);
        errno = EINVAL;
        fail("validate durable operation status");
    }
    char line[1025];
    size_t used = 0;
    while (used < (size_t)status.st_size) {
        ssize_t count = read(descriptor, line + used, (size_t)status.st_size - used);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count <= 0) {
            close(descriptor);
            close(stage_fd);
            fail("read durable operation status");
        }
        used += (size_t)count;
    }
    close(descriptor);
    close(stage_fd);
    if (used < (size_t)prefix_length || memcmp(line, prefix, (size_t)prefix_length) != 0 ||
        line[used - 1] != '\n') {
        errno = EINVAL;
        fail("operation status identity does not match query");
    }
    if (write_all_status(STDOUT_FILENO, line, used) < 0) {
        fail("write operation status response");
    }
}

static void create_file_exclusive(int root_fd, const char *stage_root,
                                  const char *relative, const char *expected,
                                  const char *expected_source_sha256,
                                  const char *operation_id) {
    struct path_parts path = split_relative(relative);
    if (path.count != 1 || strcmp(expected, "absent") != 0 ||
        !valid_operation_id(operation_id)) {
        errno = EINVAL;
        fail("create-only configuration arguments");
    }

    int stage_fd = open_absolute_directory(stage_root);
    int source_parent = open_relative_parent(stage_fd, &path);
    int source = openat(source_parent, path.items[0], O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (source < 0) {
        fail("open staged configuration without links");
    }
    struct stat source_status;
    if (fstat(source, &source_status) < 0 || !S_ISREG(source_status.st_mode)) {
        errno = EINVAL;
        fail("validate staged configuration file");
    }
    char source_digest[65];
    sha256_from_fd(source, source_digest);
    if (strcmp(source_digest, expected_source_sha256) != 0) {
        errno = ESTALE;
        fail_with_status(11, "staged configuration digest mismatch");
    }
    if (lseek(source, 0, SEEK_SET) < 0) {
        fail("rewind staged configuration");
    }

    begin_operation_status(stage_root, operation_id, "create", expected_source_sha256,
                           (uint64_t)source_status.st_size, "absent", 0);

    int destination_parent = open_relative_parent(root_fd, &path);
    struct stat existing_target;
    if (fstatat(destination_parent, path.items[0], &existing_target,
                 AT_SYMLINK_NOFOLLOW) == 0) {
        fprintf(stderr, "selected-root helper target already exists\n");
        exit(10);
    }
    if (errno != ENOENT) {
        fail("inspect final configuration target");
    }
    await_operation_commit(operation_id);
    mark_operation_commit_pending();
    test_final_open_race(destination_parent, path.items[0]);
    if (fstatat(destination_parent, path.items[0], &existing_target,
                 AT_SYMLINK_NOFOLLOW) == 0) {
        fprintf(stderr, "selected-root helper target already exists\n");
        exit(10);
    }
    if (errno != ENOENT) {
        fail("recheck final configuration target after COMMIT");
    }
    int destination = openat(destination_parent, path.items[0],
                             O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                             0644);
    if (destination < 0) {
        if (errno == EEXIST) {
            fprintf(stderr, "selected-root helper target already exists\n");
            exit(10);
        }
        fail("create configuration with exclusive final-path open");
    }
    mark_operation_mutation_started();
    test_partial_final_write(destination);

    /*
     * From this point onward, failure may leave partial bytes at the final
     * path. Never unlink or replace that path; report the outcome as
     * unverified so a later review must inspect it explicitly.
     */
    char buffer[65536];
    for (;;) {
        ssize_t read_count = read(source, buffer, sizeof(buffer));
        if (read_count == 0) {
            break;
        }
        if (read_count < 0) {
            if (errno == EINTR) {
                continue;
            }
            fail_with_status(12, "read staged configuration");
        }
        ssize_t offset = 0;
        while (offset < read_count) {
            ssize_t write_count = write(destination, buffer + offset,
                                        (size_t)(read_count - offset));
            if (write_count < 0 && errno == EINTR) {
                continue;
            }
            if (write_count <= 0) {
                fail_with_status(12, "write final configuration");
            }
            offset += write_count;
        }
    }
    if (fchmod(destination, 0644) < 0 || fsync(destination) < 0) {
        fail_with_status(12, "flush final configuration");
    }
    if (close(destination) < 0) {
        fail_with_status(12, "close final configuration");
    }

    int directory_fd = openat(destination_parent, ".",
                              O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (directory_fd < 0 || fsync(directory_fd) < 0) {
        fail_with_status(12, "flush configuration parent directory");
    }
    close(directory_fd);

    int readback = open_final_configuration_readback(
        destination_parent, path.items[0],
        O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK);
    if (readback < 0) {
        fail_with_status(12, "reopen final configuration without links");
    }
    struct stat readback_status;
    if (fstat(readback, &readback_status) < 0 ||
        !S_ISREG(readback_status.st_mode) ||
        readback_status.st_size != source_status.st_size) {
        errno = EIO;
        fail_with_status(12, "validate final configuration length");
    }
    char readback_digest[65];
    sha256_from_fd(readback, readback_digest);
    if (strcmp(readback_digest, expected_source_sha256) != 0) {
        errno = ESTALE;
        fail_with_status(12, "verify final configuration digest");
    }

    complete_operation_status(readback_digest, (uint64_t)readback_status.st_size);

    printf("CREATED %" PRIdMAX " %s write=complete flush=complete\n",
           (intmax_t)readback_status.st_size, readback_digest);
    close(readback);
    close(destination_parent);
    close(source);
    close(source_parent);
    close(stage_fd);
}

static void publish_file(int root_fd, const char *stage_root, const char *relative,
                         const char *expected, const char *expected_source_sha256) {
    struct path_parts path = split_relative(relative);
    int stage_fd = open_absolute_directory(stage_root);
    int source_parent = open_relative_parent(stage_fd, &path);
    int source = openat(source_parent, path.items[path.count - 1],
                        O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (source < 0) {
        fail("open staged file without following links");
    }
    struct stat source_status;
    if (fstat(source, &source_status) < 0 || !S_ISREG(source_status.st_mode)) {
        errno = EINVAL;
        fail("validate staged regular file");
    }
    char source_digest[65];
    if (lseek(source, 0, SEEK_SET) < 0) {
        fail("rewind staged file for digest validation");
    }
    sha256_from_fd(source, source_digest);
    if (strcmp(source_digest, expected_source_sha256) != 0) {
        errno = ESTALE;
        fail("staged file digest mismatch");
    }
    if (lseek(source, 0, SEEK_SET) < 0) {
        fail("rewind staged file for publication");
    }

    int destination_parent = open_relative_parent(root_fd, &path);
    char temporary[80];
    int destination = -1;
    for (unsigned int attempt = 0; attempt < 32; attempt++) {
        snprintf(temporary, sizeof(temporary), ".emuchef-copy-%ld-%u", (long)getpid(), attempt);
        destination = openat(destination_parent, temporary,
            O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                             source_status.st_mode & 0777);
        if (destination >= 0 || errno != EEXIST) {
            break;
        }
    }
    if (destination < 0) {
        fail("create selected-root temporary file");
    }

    char buffer[65536];
    for (;;) {
        ssize_t read_count = read(source, buffer, sizeof(buffer));
        if (read_count == 0) {
            break;
        }
        if (read_count < 0) {
            if (errno == EINTR) {
                continue;
            }
            fail("read staged file");
        }
        ssize_t offset = 0;
        while (offset < read_count) {
            ssize_t write_count = write(destination, buffer + offset,
                                        (size_t)(read_count - offset));
            if (write_count < 0 && errno == EINTR) {
                continue;
            }
            if (write_count <= 0) {
                fail("write selected-root temporary file");
            }
            offset += write_count;
        }
    }
    if (fchmod(destination, source_status.st_mode & 0777) < 0 || fsync(destination) < 0) {
        fail("flush selected-root temporary file");
    }
    char published_digest[65];
    if (lseek(destination, 0, SEEK_SET) < 0) {
        fail("rewind selected-root temporary file for digest validation");
    }
    sha256_from_fd(destination, published_digest);
    if (strcmp(published_digest, expected_source_sha256) != 0) {
        unlinkat(destination_parent, temporary, 0);
        errno = ESTALE;
        fail("staged file digest mismatch");
    }
    close(destination);
    verify_entry_precondition(destination_parent, path.items[path.count - 1], expected);
    if (renameat(destination_parent, temporary, destination_parent,
                 path.items[path.count - 1]) < 0) {
        unlinkat(destination_parent, temporary, 0);
        fail("publish selected-root file");
    }
    sync_directory(destination_parent, "flush selected-root directory after publish");

    close(source);
    close(source_parent);
    close(stage_fd);
    close(destination_parent);
    free(path.items);
    free(path.storage);
}

static void write_fd_all(int descriptor, const unsigned char *bytes, size_t length) {
    size_t offset = 0;
    while (offset < length) {
        ssize_t count = write(descriptor, bytes + offset, length - offset);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count <= 0) {
            fail("write configuration readback");
        }
        offset += (size_t)count;
    }
}

static void read_configuration_base64(int root_fd, const char *relative_path,
                                      const char *precondition) {
    static const char alphabet[] =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const size_t maximum_length = 512 * 1024;
    if (strcmp(precondition, "regular") != 0) {
        fail("invalid configuration read precondition");
    }

    struct path_parts path = split_relative(relative_path);
    if (path.count != 1 || strcmp(path.items[0], "retroarch.cfg") != 0) {
        fail("configuration read requires the reviewed RetroArch config basename");
    }
    int parent_fd = open_relative_parent(root_fd, &path);
    if (parent_fd < 0) {
        fail("open RetroArch configuration parent");
    }

    struct stat entry_status;
    if (fstatat(parent_fd, path.items[0], &entry_status, AT_SYMLINK_NOFOLLOW) < 0 ||
        !S_ISREG(entry_status.st_mode) || entry_status.st_size < 0 ||
        (uint64_t)entry_status.st_size > maximum_length) {
        close(parent_fd);
        fail("RetroArch configuration is not a bounded regular file");
    }
    int configuration_fd = openat(parent_fd, path.items[0],
                                  O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK);
    if (configuration_fd < 0) {
        close(parent_fd);
        fail("open RetroArch configuration without following symlinks");
    }
    struct stat opened_status;
    if (fstat(configuration_fd, &opened_status) < 0 || !S_ISREG(opened_status.st_mode) ||
        opened_status.st_dev != entry_status.st_dev ||
        opened_status.st_ino != entry_status.st_ino || opened_status.st_size < 0 ||
        (uint64_t)opened_status.st_size > maximum_length) {
        close(configuration_fd);
        close(parent_fd);
        fail("RetroArch configuration changed while it was opened");
    }

    size_t length = (size_t)opened_status.st_size;
    unsigned char *contents = malloc(length == 0 ? 1 : length);
    if (contents == NULL) {
        close(configuration_fd);
        close(parent_fd);
        fail("allocate RetroArch configuration readback");
    }
    size_t offset = 0;
    while (offset < length) {
        ssize_t count = read(configuration_fd, contents + offset, length - offset);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count <= 0) {
            free(contents);
            close(configuration_fd);
            close(parent_fd);
            fail("read RetroArch configuration");
        }
        offset += (size_t)count;
    }

    struct stat final_status;
    char digest[65];
    if (fstat(configuration_fd, &final_status) < 0 ||
        final_status.st_dev != opened_status.st_dev ||
        final_status.st_ino != opened_status.st_ino || final_status.st_size != opened_status.st_size ||
        fstatat(parent_fd, path.items[0], &entry_status, AT_SYMLINK_NOFOLLOW) < 0 ||
        entry_status.st_dev != opened_status.st_dev || entry_status.st_ino != opened_status.st_ino) {
        free(contents);
        close(configuration_fd);
        close(parent_fd);
        fail("RetroArch configuration changed during readback");
    }
    if (lseek(configuration_fd, 0, SEEK_SET) < 0) {
        free(contents);
        close(configuration_fd);
        close(parent_fd);
        fail("rewind RetroArch configuration before digest verification");
    }
    sha256_from_fd(configuration_fd, digest);

    char header[96];
    int header_length = snprintf(header, sizeof(header), "READ %zu %s\n", length, digest);
    if (header_length <= 0 || (size_t)header_length >= sizeof(header)) {
        free(contents);
        close(configuration_fd);
        close(parent_fd);
        fail("format RetroArch configuration readback");
    }
    write_fd_all(STDOUT_FILENO, (const unsigned char *)header, (size_t)header_length);
    for (size_t index = 0; index < length; index += 3) {
        unsigned int value = (unsigned int)contents[index] << 16;
        if (index + 1 < length) {
            value |= (unsigned int)contents[index + 1] << 8;
        }
        if (index + 2 < length) {
            value |= (unsigned int)contents[index + 2];
        }
        unsigned char encoded[4] = {
            (unsigned char)alphabet[(value >> 18) & 0x3f],
            (unsigned char)alphabet[(value >> 12) & 0x3f],
            (unsigned char)(index + 1 < length ? alphabet[(value >> 6) & 0x3f] : '='),
            (unsigned char)(index + 2 < length ? alphabet[value & 0x3f] : '='),
        };
        write_fd_all(STDOUT_FILENO, encoded, sizeof(encoded));
    }
    write_fd_all(STDOUT_FILENO, (const unsigned char *)"\n", 1);
    free(contents);
    close(configuration_fd);
    close(parent_fd);
}

/* Keep canonical path and selected identity in separate argv entries. */
static int valid_operation_id(const char *operation_id) {
    static const char allowed[] =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    size_t length = strlen(operation_id);
    return length > 0 && length <= 128 &&
           strspn(operation_id, allowed) == length;
}

static void await_operation_commit(const char *operation_id) {
    if (!valid_operation_id(operation_id)) {
        fail("invalid operation identity for READY/COMMIT");
    }
    char expected[160];
    char received[160];
    int length = snprintf(expected, sizeof(expected), "COMMIT %s\n", operation_id);
    if (length < 0 || (size_t)length >= sizeof(expected)) {
        fail("format operation commit frame");
    }
    printf("READY %s\n", operation_id);
    if (fflush(stdout) != 0) {
        fail("flush operation READY frame");
    }
    if (fgets(received, sizeof(received), stdin) == NULL ||
        strcmp(received, expected) != 0) {
        fail("operation COMMIT frame did not match READY identity");
    }
}

static unsigned char *read_verified_staged_configuration(
    const char *stage_root, const char *expected_source_sha256,
    size_t *source_length) {
    int stage_fd = open(stage_root, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    if (stage_fd < 0) {
        fail("open staged configuration directory");
    }
    int source_fd = openat(stage_fd, "retroarch.cfg",
                           O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (source_fd < 0) {
        close(stage_fd);
        fail("open staged RetroArch configuration");
    }
    struct stat source_status;
    if (fstat(source_fd, &source_status) < 0 || !S_ISREG(source_status.st_mode) ||
        source_status.st_size < 0 || source_status.st_size > 524288) {
        close(source_fd);
        close(stage_fd);
        fail("staged RetroArch configuration is not a bounded regular file");
    }

    *source_length = (size_t)source_status.st_size;
    unsigned char *contents = malloc(*source_length == 0 ? 1 : *source_length);
    if (contents == NULL) {
        close(source_fd);
        close(stage_fd);
        fail("allocate staged RetroArch configuration");
    }
    size_t offset = 0;
    while (offset < *source_length) {
        ssize_t count = read(source_fd, contents + offset, *source_length - offset);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count <= 0) {
            free(contents);
            close(source_fd);
            close(stage_fd);
            fail("read staged RetroArch configuration");
        }
        offset += (size_t)count;
    }

    char actual_source_sha256[65];
    if (lseek(source_fd, 0, SEEK_SET) < 0) {
        free(contents);
        close(source_fd);
        close(stage_fd);
        fail("rewind staged RetroArch configuration");
    }
    sha256_from_fd(source_fd, actual_source_sha256);
    if (strcmp(actual_source_sha256, expected_source_sha256) != 0) {
        free(contents);
        close(source_fd);
        close(stage_fd);
        fail("staged RetroArch configuration digest changed");
    }
    close(source_fd);
    close(stage_fd);
    return contents;
}

static void replace_configuration_in_place(
    int root_fd, const char *stage_root, const char *relative_path,
    const char *expected_original_sha256, const char *expected_original_length,
    const char *expected_source_sha256, const char *operation_id) {
    char *length_end = NULL;
    errno = 0;
    unsigned long long parsed_original_length =
        strtoull(expected_original_length, &length_end, 10);
    if (errno != 0 || length_end == expected_original_length || *length_end != '\0' ||
        parsed_original_length > 524288 || strlen(expected_original_sha256) != 64 ||
        strlen(expected_source_sha256) != 64) {
        fail("invalid RetroArch configuration replacement precondition");
    }

    struct path_parts path = split_relative(relative_path);
    if (path.count == 0) {
        free(path.storage);
        fail("invalid RetroArch configuration replacement path");
    }
    const char *entry_name = path.items[path.count - 1];
    int parent_fd = open_relative_parent(root_fd, &path);
    if (parent_fd < 0) {
        free(path.storage);
        fail("open RetroArch configuration parent for replacement");
    }

    size_t source_length = 0;
    unsigned char *source_contents = NULL;
    int target_fd = -1;
    struct stat target_status;
    struct stat named_status;
    char actual_original_sha256[65];
    char actual_destination_sha256[65];

    if (fstatat(parent_fd, entry_name, &named_status, AT_SYMLINK_NOFOLLOW) < 0 ||
        !S_ISREG(named_status.st_mode)) {
        close(parent_fd);
        free(path.storage);
        fail("RetroArch configuration replacement target is not a regular file");
    }
    target_fd = openat(parent_fd, entry_name, O_RDWR | O_NOFOLLOW | O_CLOEXEC);
    if (target_fd < 0 || fstat(target_fd, &target_status) < 0 ||
        !S_ISREG(target_status.st_mode) || target_status.st_dev != named_status.st_dev ||
        target_status.st_ino != named_status.st_ino || target_status.st_size < 0 ||
        (unsigned long long)target_status.st_size != parsed_original_length) {
        if (target_fd >= 0) {
            close(target_fd);
        }
        close(parent_fd);
        free(path.storage);
        fail("RetroArch configuration replacement target changed");
    }
    sha256_from_fd(target_fd, actual_original_sha256);
    if (strcmp(actual_original_sha256, expected_original_sha256) != 0 ||
        lseek(target_fd, 0, SEEK_SET) < 0) {
        close(target_fd);
        close(parent_fd);
        free(path.storage);
        fail("RetroArch configuration original-file precondition failed");
    }

    source_contents = read_verified_staged_configuration(
        stage_root, expected_source_sha256, &source_length);
    begin_operation_status(stage_root, operation_id, "replace",
                           expected_source_sha256, (uint64_t)source_length,
                           expected_original_sha256, (uint64_t)parsed_original_length);
    await_operation_commit(operation_id);
    mark_operation_commit_pending();

    if (fstatat(parent_fd, entry_name, &named_status, AT_SYMLINK_NOFOLLOW) < 0 ||
        !S_ISREG(named_status.st_mode) || fstat(target_fd, &target_status) < 0 ||
        named_status.st_dev != target_status.st_dev ||
        named_status.st_ino != target_status.st_ino || target_status.st_size < 0 ||
        (unsigned long long)target_status.st_size != parsed_original_length) {
        free(source_contents);
        close(target_fd);
        close(parent_fd);
        free(path.storage);
        fail("RetroArch configuration changed while awaiting COMMIT");
    }
    sha256_from_fd(target_fd, actual_original_sha256);
    if (strcmp(actual_original_sha256, expected_original_sha256) != 0 ||
        (mark_operation_mutation_started(), ftruncate(target_fd, 0) < 0) ||
        lseek(target_fd, 0, SEEK_SET) < 0) {
        free(source_contents);
        close(target_fd);
        close(parent_fd);
        free(path.storage);
        fail("RetroArch configuration replacement precondition failed at COMMIT");
    }

    write_fd_all(target_fd, source_contents, source_length);
    free(source_contents);
    if (fsync(target_fd) < 0 || lseek(target_fd, 0, SEEK_SET) < 0) {
        close(target_fd);
        close(parent_fd);
        free(path.storage);
        fail("flush replaced RetroArch configuration");
    }
    sha256_from_fd(target_fd, actual_destination_sha256);
    if (fstat(target_fd, &target_status) < 0 || !S_ISREG(target_status.st_mode) ||
        target_status.st_size < 0 || (size_t)target_status.st_size != source_length ||
        strcmp(actual_destination_sha256, expected_source_sha256) != 0 ||
        fstatat(parent_fd, entry_name, &named_status, AT_SYMLINK_NOFOLLOW) < 0 ||
        named_status.st_dev != target_status.st_dev ||
        named_status.st_ino != target_status.st_ino) {
        close(target_fd);
        close(parent_fd);
        free(path.storage);
        fail("replaced RetroArch configuration readback did not verify");
    }

    complete_operation_status(actual_destination_sha256,
                              (uint64_t)target_status.st_size);

    printf("REPLACED %zu %s write=complete flush=complete\n", source_length,
           actual_destination_sha256);
    fflush(stdout);
    close(target_fd);
    close(parent_fd);
    free(path.storage);
}

int main(int argc, char **argv) {
    int valid_request =
        (argc == 8 && (strcmp(argv[5], "read-file") == 0 ||
                       strcmp(argv[5], "ensure-dir") == 0 ||
                       strcmp(argv[5], "remove-file") == 0 ||
                       strcmp(argv[5], "remove-dir") == 0 ||
                       strcmp(argv[5], "operation-status") == 0)) ||
        (argc == 9 && strcmp(argv[5], "publish") == 0) ||
        (argc == 10 && strcmp(argv[5], "create-file") == 0) ||
        (argc == 11 && strcmp(argv[5], "replace-file") == 0);
    if (!valid_request) {
        fprintf(stderr, "usage: helper ROOT CANONICAL_PATH SELECTED_IDENTITY STAGE_ROOT ACTION RELATIVE_PATH PRECONDITION [SOURCE_SHA256]\n");
        return 64;
    }
    int selected_root_fd = open_reviewed_selected_root(argv[1], argv[2]);
    verify_selected_identity(selected_root_fd, argv[3]);

    if (strcmp(argv[5], "read-file") == 0) {
        read_configuration_base64(selected_root_fd, argv[6], argv[7]);
    } else if (strcmp(argv[5], "ensure-dir") == 0) {
        ensure_directory(selected_root_fd, argv[6], argv[7]);
    } else if (strcmp(argv[5], "remove-file") == 0) {
        remove_entry(selected_root_fd, argv[6], false, argv[7]);
    } else if (strcmp(argv[5], "remove-dir") == 0) {
        remove_entry(selected_root_fd, argv[6], true, argv[7]);
    } else if (strcmp(argv[5], "operation-status") == 0) {
        if (strcmp(argv[7], "v1") != 0) {
            errno = EINVAL;
            fail("unsupported operation status version");
        }
        query_operation_status(argv[4], argv[6]);
    } else if (strcmp(argv[5], "publish") == 0) {
        publish_file(selected_root_fd, argv[4], argv[6], argv[7], argv[8]);
} else if (strcmp(argv[5], "create-file") == 0) {
create_file_exclusive(selected_root_fd, argv[4], argv[6], argv[7], argv[8], argv[9]);
} else if (strcmp(argv[5], "replace-file") == 0) {
    replace_configuration_in_place(selected_root_fd, argv[4], argv[6], argv[7],
                                   argv[8], argv[9], argv[10]);
    } else {
        fprintf(stderr, "selected-root helper rejected unsupported action\n");
        close(selected_root_fd);
        return 64;
    }
    close(selected_root_fd);
    return 0;
}
