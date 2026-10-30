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

static void fail(const char *action) {
    fprintf(stderr, "selected-root helper %s: %s\n", action, strerror(errno));
    exit(1);
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

static void verify_binding_canonical_path(const char *binding, const char *expected) {
    const char *separator = strchr(binding, '|');
    if (separator == NULL) {
        errno = EINVAL;
        fail("reviewed destination binding validation");
    }
    size_t length = (size_t)(separator - binding);
    if (length != strlen(expected) || strncmp(binding, expected, length) != 0) {
        errno = ESTALE;
        fail("reviewed canonical destination changed");
    }
}

/* Resolve only the reviewed Android shared-storage aliases; descendants stay no-follow. */
static int open_reviewed_selected_root(const char *path, const char *binding) {
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
        verify_binding_canonical_path(binding, path);
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
    verify_binding_canonical_path(binding, expected);

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

static void verify_selected_identity(int root_fd, const char *binding) {
    const char *selected = strstr(binding, "|selected:");
    if (selected == NULL) {
        errno = EINVAL;
        fail("reviewed destination binding validation");
    }
    selected += strlen("|selected:");
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

int main(int argc, char **argv) {
    if ((argc != 7 && argc != 8) ||
        (argc == 8 && strcmp(argv[4], "publish") != 0) ||
        (argc == 7 && strcmp(argv[4], "publish") == 0)) {
        fprintf(stderr, "usage: helper ROOT BINDING STAGE_ROOT ACTION RELATIVE_PATH PRECONDITION [SOURCE_SHA256]\n");
        return 64;
    }
    int selected_root_fd = open_reviewed_selected_root(argv[1], argv[2]);
    verify_selected_identity(selected_root_fd, argv[2]);

    if (strcmp(argv[4], "ensure-dir") == 0) {
        ensure_directory(selected_root_fd, argv[5], argv[6]);
    } else if (strcmp(argv[4], "remove-file") == 0) {
        remove_entry(selected_root_fd, argv[5], false, argv[6]);
    } else if (strcmp(argv[4], "remove-dir") == 0) {
        remove_entry(selected_root_fd, argv[5], true, argv[6]);
    } else if (strcmp(argv[4], "publish") == 0) {
        publish_file(selected_root_fd, argv[3], argv[5], argv[6], argv[7]);
    } else {
        fprintf(stderr, "selected-root helper rejected unsupported action\n");
        close(selected_root_fd);
        return 64;
    }
    close(selected_root_fd);
    return 0;
}
