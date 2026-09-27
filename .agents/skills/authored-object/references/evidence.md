# Authoring evidence

Store retained authoring evidence under `.chatgpt/authoring-evidence/` using an ignored local filename such as `<timestamp>-<scope>.local.md`.

## Evidence strength

Classify material technical facts as:

- **observed/verified**: directly established by authoritative local inspection, connected-device observation, APK inspection, or strong primary evidence;
- **derived**: deterministically inferred from verified facts;
- **suggested**: conservative proposal grounded in current EmuChef semantics but not directly verified;
- **user-supplied**: explicitly provided by the user but not independently verified;
- **missing**: needed fact is unavailable;
- **conflicting**: credible sources disagree materially.

Prefer primary upstream sources. When using the web, record source title/URL, access date, supported fact, and evidence strength. Label secondary/community evidence explicitly.

## Never persist transient authority

Do not persist:

- credentials, tokens, secrets, or authentication material;
- exact ADB serials;
- native absolute user-file paths when a sanitized description is enough;
- raw transient handles or other process-local authority;
- sensitive values that are unnecessary for later authoring/diagnosis.

## Evidence note contents

Record:

1. user intent;
2. affected authored IDs/files;
3. important evidence and provenance;
4. observed/verified, derived, suggested, user-supplied, missing, and conflicting facts;
5. explicit product decisions;
6. important rejected alternatives when they materially shaped behavior;
7. validation choices and results;
8. physical target in sanitized form and result when physical validation was selected;
9. unresolved follow-up, especially references outside `authored/**` that the skill was not allowed to modify.
