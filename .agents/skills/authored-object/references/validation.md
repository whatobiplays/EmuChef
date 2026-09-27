# Optional validation

Validation is opt-in. Offer relevant choices during alignment unless the request already selected or declined them.

## Static validation

When approved:

1. inspect the repository's current validation and generator interfaces;
2. choose the narrowest sufficient repo-owned checks for the changed authored objects;
3. prefer typed parsing/schema checks, canonical emission where applicable, catalog/reference validation, collision checks, planner admission, or other current authored-data checks over a blanket full build;
4. do not hard-code a validation command into the skill when the repo can reveal the current authority;
5. if validation finds an authored defect, repair and rerun automatically only when the correction stays within approved semantics;
6. return to alignment when the fix requires new behavior, broader mutation, another object, or another product decision.

If validation is declined, still inspect the authored diff for obvious structural/reference mistakes. Report the result as not repository-validated.

## Physical validation

Only recipe and device-plan create/edit workflows offer physical execution as part of this family.

When approved:

- choose a target only when applicability and required capabilities are sufficiently established;
- if zero or multiple suitable targets exist, ask the user to resolve target selection;
- align the sanitized target identity, expected mutations, required user-owned inputs, and expected postconditions before execution;
- after that alignment, do not add a redundant second confirmation;
- use EmuChef's production planning/review/execution path rather than ad-hoc ADB commands;
- permit production admission/planning/preflight even when the separate static-validation option was declined;
- do not perform out-of-band cleanup/rollback afterward;
- require successful execution plus meaningful postcondition verification where the authored model supports it.

Environmental failures such as disconnects, network failure, missing user input, or unsuitable device state are not authored defects. Preserve the authored artifact and report physical validation as incomplete/failed for the external reason.
