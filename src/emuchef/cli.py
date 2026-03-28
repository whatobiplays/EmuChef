"""CLI entrypoints for the current vertical slice."""

from __future__ import annotations

import argparse
import logging
import sys
from collections.abc import Mapping, Sequence
from dataclasses import replace
from pathlib import Path

from emuchef.domain import (
    Availability,
    DeviceContext,
    DraftPlan,
    DraftUpdateResult,
    PlanningResult,
    PlanningStatus,
    ValidationResult,
    WarningCode,
    WarningMessage,
)
from emuchef.executor import (
    AdbResolutionError,
    DetectedDevice,
    DryRunAdb,
    ExecutionProgressEvent,
    ExecutorRunner,
    ProgressPhase,
    SubprocessAdb,
    resolve_adb_executable,
)
from emuchef.io import (
    dump_yaml,
    load_authored_catalog,
    load_execution_plan_file,
    validate_authored_catalog,
    validate_authored_path,
)
from emuchef.io.serde import load_yaml
from emuchef.planner import (
    BindInput,
    CatalogLoadError,
    DeselectRecipe,
    DeselectStep,
    Planner,
    ProfileMatchFacts,
    SelectRecipe,
    SelectStep,
    UnbindInput,
    match_device_profile,
    match_device_profiles,
)

logger = logging.getLogger(__name__)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="emuchef")
    subparsers = parser.add_subparsers(dest="command", required=True)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--verbose", action="store_true", help="Emit the full structured artifact and enable info logs.")
    common.add_argument("--debug", action="store_true", help="Enable debug logging.")
    common.add_argument("--adb", help="Explicit path to the ADB executable.")

    device_context = argparse.ArgumentParser(add_help=False)
    device_context.add_argument("--serial", help="ADB serial to use for detection or execution.")
    device_context.add_argument("--manufacturer")
    device_context.add_argument("--model")
    device_context.add_argument("--android-version", type=int)
    device_context.add_argument("--device-tag", action="append", default=[])

    authored = argparse.ArgumentParser(add_help=False)
    authored.add_argument("--authored-root", default="authored")
    authored.add_argument("--device-plan", required=True)
    authored.add_argument("--ops", help="YAML file containing ordered planner operations to replay.")
    authored.add_argument("--bind", action="append", default=[], metavar="REF=VALUE")

    draft_parser = subparsers.add_parser(
        "draft",
        parents=[common, authored, device_context],
        help="Build the current draft plan and print a concise summary or structured artifact.",
    )

    plan_parser = subparsers.add_parser(
        "plan",
        parents=[common, authored, device_context],
        help="Emit a planning_result for the current draft state.",
    )
    plan_parser.add_argument("--output", help="Optional file path for the structured planning_result YAML.")

    detect_parser = subparsers.add_parser(
        "detect",
        parents=[common],
        help="Detect basic device facts from ADB.",
    )
    detect_parser.add_argument("--serial")

    detect_profiles_parser = subparsers.add_parser(
        "detect-profiles",
        parents=[common],
        help="Detect the connected device and evaluate all authored device profiles against it.",
    )
    detect_profiles_parser.add_argument("--authored-root", default="authored")
    detect_profiles_parser.add_argument("--serial")

    validate_parser = subparsers.add_parser(
        "validate",
        parents=[common],
        help="Validate authored YAML files or a full authored catalog.",
    )
    validate_parser.add_argument("path", nargs="?", help="Optional single YAML file path to validate.")
    validate_parser.add_argument("--authored-root")

    apply_parser = subparsers.add_parser(
        "apply",
        parents=[common],
        help="Execute an emitted execution_plan or planning_result.",
    )
    apply_parser.add_argument("--plan-file", required=True)
    apply_parser.add_argument("--serial")
    apply_parser.add_argument("--dry-run", action="store_true")

    args = parser.parse_args(argv)
    _configure_logging(args)

    try:
        setattr(
            args,
            "_resolved_adb",
            resolve_adb_executable(
                cli_value=getattr(args, "adb", None),
                config_value=_configured_adb_executable(),
            ),
        )
        if args.command == "draft":
            return _run_draft(args)
        if args.command == "plan":
            return _run_plan(args)
        if args.command == "detect":
            return _run_detect(args)
        if args.command == "detect-profiles":
            return _run_detect_profiles(args)
        if args.command == "validate":
            return _run_validate(args)
        if args.command == "apply":
            return _run_apply(args)
    except CatalogLoadError as exc:
        sys.stderr.write(_format_catalog_load_error(exc))
        return 1
    except AdbResolutionError as exc:
        logger.debug("CLI command failed", exc_info=True)
        sys.stderr.write(f"{exc.code.value}: {exc}\n")
        return 1
    except ValueError as exc:
        logger.debug("CLI command failed", exc_info=True)
        sys.stderr.write(f"Error: {exc}\n")
        return 1
    return 1


def _run_draft(args: argparse.Namespace) -> int:
    catalog, session, detected_device = _build_session(args)
    maybe_failure = _replay_ops_and_bindings(session, args.ops, args.bind)
    if maybe_failure is not None:
        return _write_failure_result(maybe_failure, verbose=args.verbose)

    draft_plan = _append_profile_mismatch_warning_to_draft(
        catalog=catalog,
        draft_plan=session.draft_plan,
        detected_device=detected_device,
    )
    if args.verbose:
        sys.stdout.write(dump_yaml(draft_plan))
        return 0

    sys.stdout.write(_format_draft_summary(draft_plan))
    return 0


def _run_plan(args: argparse.Namespace) -> int:
    catalog, session, detected_device = _build_session(args)
    maybe_failure = _replay_ops_and_bindings(session, args.ops, args.bind)
    if maybe_failure is not None:
        return _write_failure_result(maybe_failure, verbose=args.verbose)

    result = _append_profile_mismatch_warning_to_result(
        catalog=catalog,
        result=session.emit_execution_plan(),
        detected_device=detected_device,
    )
    payload = dump_yaml(result, path=args.output) if args.output else dump_yaml(result)
    if args.verbose:
        sys.stdout.write(payload)
    else:
        sys.stdout.write(_format_planning_summary(result, output_path=args.output))
    return 0 if result.execution_plan is not None else 1


def _run_detect(args: argparse.Namespace) -> int:
    detected = _build_adb(args).detect_device()
    if args.verbose:
        sys.stdout.write(dump_yaml(detected))
        return 0
    sys.stdout.write(_format_detect_summary(detected))
    return 0


def _run_detect_profiles(args: argparse.Namespace) -> int:
    catalog = load_authored_catalog(args.authored_root)
    detected = _build_adb(args).detect_device()
    matches = match_device_profiles(catalog.device_profiles.values(), _profile_match_facts(detected))
    if args.verbose:
        sys.stdout.write(
            dump_yaml(
                {
                    "device": detected,
                    "profiles": matches,
                }
            )
        )
        return 0
    sys.stdout.write(_format_detect_profiles_summary(detected, matches))
    return 0


def _run_apply(args: argparse.Namespace) -> int:
    plan_path = Path(args.plan_file)
    execution_plan = load_execution_plan_file(plan_path)
    adb = DryRunAdb() if args.dry_run else _build_adb(args)
    runner = ExecutorRunner(adb=adb, workdir=plan_path.parent, sleep_fn=(lambda _: None) if args.dry_run else None)
    result = runner.run(
        execution_plan,
        progress_callback=_make_execution_progress_callback(verbose=args.verbose, dry_run=args.dry_run),
    )
    sys.stdout.write(_format_execution_summary(result, dry_run=args.dry_run))
    return 0 if result.success else 1


def _run_validate(args: argparse.Namespace) -> int:
    result = _validate_target(args)
    if args.verbose:
        sys.stdout.write(dump_yaml(result))
    else:
        sys.stdout.write(_format_validation_summary(result))
    return 0 if result.status.value != "error" else 1


def _build_session(args: argparse.Namespace):
    catalog = load_authored_catalog(args.authored_root)
    planner = Planner(catalog)
    device_context, detected_device = _resolve_device_context(catalog, args)
    return catalog, planner.start_session(device_plan_ref=args.device_plan, device_context=device_context), detected_device


def _validate_target(args: argparse.Namespace) -> ValidationResult:
    if args.path:
        target = Path(args.path)
        if target.is_dir():
            return validate_authored_catalog(target)
        return validate_authored_path(target, authored_root=args.authored_root if args.authored_root else None)
    return validate_authored_catalog(args.authored_root or "authored")


def _resolve_device_context(catalog, args: argparse.Namespace) -> tuple[DeviceContext, DetectedDevice | None]:
    device_plan = catalog.device_plans.get(args.device_plan)
    if device_plan is None:
        raise ValueError(_unknown_device_plan_message(catalog, args.device_plan))
    device_profile = catalog.device_profiles[device_plan.device_profile_ref]

    explicit_tags = tuple(args.device_tag)
    explicit_manufacturer = args.manufacturer
    explicit_model = args.model
    explicit_android_version = args.android_version

    detected: DetectedDevice | None = None
    resolved_adb = args._resolved_adb
    try:
        detected = SubprocessAdb(serial=args.serial, executable=resolved_adb).detect_device()
        logger.info("Using detected device facts from %s", detected.serial)
    except AdbResolutionError as exc:
        if resolved_adb != "adb":
            raise
        logger.info("ADB device detection unavailable, using device-plan defaults: %s", exc)
    except ValueError as exc:
        logger.info("ADB device detection unavailable, using device-plan defaults: %s", exc)

    manufacturer = explicit_manufacturer or (detected.manufacturer if detected is not None else None)
    if not manufacturer:
        manufacturer = device_profile.match.manufacturer_contains[0] if device_profile.match.manufacturer_contains else device_profile.name

    model = explicit_model or (detected.model if detected is not None else None) or device_profile.name

    android_version = explicit_android_version
    if android_version is None and detected is not None and detected.android_version > 0:
        android_version = detected.android_version
    if android_version is None:
        android_version = device_profile.match.android_version.min if device_profile.match.android_version else None
    if android_version is None:
        android_version = 0

    return (
        DeviceContext(
            manufacturer=manufacturer,
            model=model,
            android_version=android_version,
            android_api_level=detected.android_api_level if detected is not None else None,
            device_tags=explicit_tags,
        ),
        detected,
    )


def _unknown_device_plan_message(catalog, requested_ref: str) -> str:
    available_plan_ids = ", ".join(sorted(catalog.device_plans))
    profile_matches = [
        plan_id
        for plan_id, plan in catalog.device_plans.items()
        if plan.device_profile_ref == requested_ref
    ]
    if profile_matches:
        return (
            f"Unknown device plan: {requested_ref}. "
            f"{requested_ref!r} is a device profile, not a device plan. "
            f"Matching device plans: {', '.join(sorted(profile_matches))}. "
            f"Available device plans: {available_plan_ids}"
        )
    return f"Unknown device plan: {requested_ref}. Available device plans: {available_plan_ids}"


def _replay_ops_and_bindings(session, ops_path: str | None, bindings: Sequence[str]) -> DraftUpdateResult | None:
    if ops_path:
        operations = _load_operations_file(ops_path)
        for index, operation in enumerate(operations, start=1):
            logger.info("Applying operation %d/%d: %s", index, len(operations), operation.type.value)
            update = session.apply(operation)
            if update.errors:
                logger.debug("Planner operation failed at index %d", index)
                return update

    for ref, value in _parse_bindings(list(bindings)).items():
        update = session.bind_input(ref, value)
        if update.errors:
            return update

    return None


def _load_operations_file(path: str | Path):
    raw = load_yaml(path)
    if isinstance(raw, Mapping):
        raw = raw.get("operations", raw.get("ops", raw))
    if not isinstance(raw, list):
        raise ValueError("Ops file must contain a list of planner operations or an operations/ops list.")

    operations = []
    for index, item in enumerate(raw, start=1):
        if not isinstance(item, Mapping):
            raise ValueError(f"Operation #{index} must be a mapping.")
        operation_type = str(item.get("type", ""))
        if operation_type == "select_recipe":
            operations.append(SelectRecipe(recipe_ref=str(item["recipe_ref"])))
        elif operation_type == "deselect_recipe":
            operations.append(DeselectRecipe(recipe_ref=str(item["recipe_ref"])))
        elif operation_type == "select_step":
            operations.append(SelectStep(step_id=str(item["step_id"])))
        elif operation_type == "deselect_step":
            operations.append(DeselectStep(step_id=str(item["step_id"])))
        elif operation_type == "bind_input":
            operations.append(BindInput(input_id=str(item["input_id"]), value=item.get("value")))
        elif operation_type == "unbind_input":
            operations.append(UnbindInput(input_id=str(item["input_id"])))
        else:
            raise ValueError(f"Unsupported operation type at index {index}: {operation_type!r}")
    return operations


def _write_failure_result(update: DraftUpdateResult, verbose: bool) -> int:
    if verbose:
        sys.stdout.write(dump_yaml(update))
        return 1

    sys.stderr.write(_format_update_error_summary(update))
    return 1


def _format_detect_summary(device: DetectedDevice) -> str:
    return (
        f"Serial: {device.serial}\n"
        f"Manufacturer: {device.manufacturer}\n"
        f"Model: {device.model}\n"
        f"Android version: {device.android_version}\n"
        f"Root available: {'yes' if device.root_available else 'no'}\n"
    )


def _format_draft_summary(draft: DraftPlan) -> str:
    selected_recipes = [recipe.id for recipe in draft.recipes if recipe.selected and not recipe.auto_included]
    auto_included = [recipe.id for recipe in draft.recipes if recipe.auto_included]
    selected_steps = [step.id for step in draft.steps if step.selected]
    unavailable_steps = [
        f"{step.id}: {step.reason.message if step.reason is not None else 'Unavailable'}"
        for step in draft.steps
        if step.availability is Availability.UNAVAILABLE
    ]
    unresolved_inputs = [
        f"{item.id} ({item.label})" for item in draft.inputs if item.required and not item.resolved
    ]

    lines = [
        f"Draft: {draft.id}",
        f"Device plan: {draft.source.device_plan_ref}",
        "Selected recipes:",
        *_bullet_lines(selected_recipes),
        "Auto-included recipes:",
        *_bullet_lines(auto_included),
        "Selected steps:",
        *_bullet_lines(selected_steps),
        "Unavailable steps:",
        *_bullet_lines(unavailable_steps),
        "Unresolved required inputs:",
        *_bullet_lines(unresolved_inputs),
    ]
    if draft.warnings:
        lines.extend(
            [
                "Warnings:",
                *_bullet_lines([f"{warning.code.value}: {warning.message}" for warning in draft.warnings]),
            ]
        )
    return "\n".join(lines) + "\n"


def _format_detect_profiles_summary(
    device: DetectedDevice,
    matches,
) -> str:
    matching_profile_ids = [result.profile_id for result in matches if result.matched]
    lines = [
        f"Detected device: {device.manufacturer} {device.model} (Android {device.android_version})",
        "Matching device profiles:",
        *_bullet_lines(matching_profile_ids),
    ]
    return "\n".join(lines) + "\n"


def _format_planning_summary(result: PlanningResult, output_path: str | None = None) -> str:
    lines = [f"Planning status: {result.status.value}"]
    if output_path:
        lines.append(f"Wrote planning result: {Path(output_path).resolve()}")
    if result.execution_plan is not None:
        lines.append(f"Execution plan: {result.execution_plan.id}")
        lines.append("Runnable steps:")
        lines.extend(_bullet_lines([step.id for step in result.execution_plan.steps]))
    if result.warnings:
        lines.append("Warnings:")
        lines.extend(_bullet_lines([f"{warning.code.value}: {warning.message}" for warning in result.warnings]))
    if result.errors:
        lines.append("Errors:")
        lines.extend(_bullet_lines([f"{error.code.value}: {error.message}" for error in result.errors]))
    return "\n".join(lines) + "\n"


def _format_execution_summary(result, dry_run: bool) -> str:
    prefix = "Dry run" if dry_run else "Execution"
    succeeded = sum(1 for record in result.steps if record.status.value == "executed")
    skipped = sum(1 for record in result.steps if record.status.value == "skipped")
    failed = sum(1 for record in result.steps if record.status.value == "failed")
    not_run = max(result.total_steps - len(result.steps), 0)
    lines = [
        f"{prefix}: {'success' if result.success else 'failed'}",
        f"- total: {result.total_steps}",
        f"- succeeded: {succeeded}",
        f"- skipped: {skipped}",
        f"- failed: {failed}",
        f"- not run: {not_run}",
    ]
    if result.permission_results:
        permission_executed = sum(1 for record in result.permission_results if record.status.value == "executed")
        permission_skipped = sum(1 for record in result.permission_results if record.status.value == "skipped")
        permission_failed = sum(1 for record in result.permission_results if record.status.value == "failed")
        permission_manual = sum(1 for record in result.permission_results if record.status.value == "manual_required")
        lines.extend(
            [
                "Permission actions:",
                f"- executed: {permission_executed}",
                f"- skipped: {permission_skipped}",
                f"- failed: {permission_failed}",
                f"- manual_required: {permission_manual}",
                *_bullet_lines([_format_permission_result(record) for record in result.permission_results]),
            ]
        )
    return "\n".join(lines) + "\n"


def _format_update_error_summary(update: DraftUpdateResult) -> str:
    lines = ["Draft update failed:"]
    lines.extend(_bullet_lines([f"{error.code.value}: {error.message}" for error in update.errors]))
    return "\n".join(lines) + "\n"


def _format_catalog_load_error(exc: CatalogLoadError) -> str:
    lines = ["Catalog load failed:"]
    lines.extend(_bullet_lines([f"{error.code.value}: {error.message}" for error in exc.errors]))
    return "\n".join(lines) + "\n"


def _format_validation_summary(result: ValidationResult) -> str:
    lines = [f"Validation status: {result.status.value}", "Validated paths:"]
    lines.extend(_bullet_lines(list(result.validated_paths)))
    grouped_issues = _group_validation_issues(result)
    if grouped_issues:
        lines.append("Issues:")
        for file_path, issues in grouped_issues.items():
            lines.append(file_path)
            for issue in issues:
                lines.append(f"  - {issue.code.value}: {issue.message}")
                field = issue.details.get("field")
                if field:
                    lines.append(f"    field: {field}")
    return "\n".join(lines) + "\n"


def _group_validation_issues(result: ValidationResult) -> dict[str, list[object]]:
    grouped: dict[str, list[object]] = {}
    for issue in (*result.warnings, *result.errors):
        file_path = str(issue.details.get("file") or "(unknown file)")
        grouped.setdefault(file_path, []).append(issue)
    return grouped


def _bullet_lines(items: Sequence[str]) -> list[str]:
    if not items:
        return ["- (none)"]
    return [f"- {item}" for item in items]


def _format_permission_result(record) -> str:
    action_name = record.permission or record.op or record.manual_type or record.kind
    detail = f"{record.kind} {record.package_name} {action_name}".strip()
    provenance = f"{record.step_id} -> {record.source_recipe_id}:{record.source_section}"
    if record.message:
        return f"{record.status.value}: {detail} ({provenance}) - {record.message}"
    return f"{record.status.value}: {detail} ({provenance})"


def _configure_logging(args: argparse.Namespace) -> None:
    level = logging.WARNING
    if getattr(args, "debug", False):
        level = logging.DEBUG
    elif getattr(args, "verbose", False):
        level = logging.INFO
    logging.basicConfig(level=level, format="%(levelname)s %(name)s: %(message)s", force=True)


def _build_adb(args: argparse.Namespace) -> SubprocessAdb:
    return SubprocessAdb(serial=args.serial, executable=args._resolved_adb)


def _configured_adb_executable() -> str | None:
    # Hook for future config plumbing.
    return None


def _parse_bindings(values: list[str]) -> dict[str, object]:
    grouped: dict[str, list[str]] = {}
    for item in values:
        if "=" not in item:
            raise ValueError(f"Invalid --bind value: {item!r}. Expected REF=VALUE.")
        ref, raw_value = item.split("=", 1)
        grouped.setdefault(ref, []).append(raw_value)
    return {
        ref: raw_values[0] if len(raw_values) == 1 else raw_values
        for ref, raw_values in grouped.items()
    }


def _make_execution_progress_callback(verbose: bool, dry_run: bool):
    def callback(event: ExecutionProgressEvent) -> None:
        sys.stdout.write(_format_execution_progress_event(event, verbose=verbose, dry_run=dry_run))
        sys.stdout.flush()

    return callback


def _format_execution_progress_event(event: ExecutionProgressEvent, *, verbose: bool, dry_run: bool) -> str:
    if event.phase is ProgressPhase.FINISHED:
        status = event.status.value if event.status is not None else "finished"
        suffix = f" ({event.message})" if verbose and event.message else ""
        label = f"{event.step_name} [{event.step_id}]" if verbose else event.step_name
        return f"[{event.step_index}/{event.total_steps}] {label}: {status}{suffix}\n"

    phase_label = {
        ProgressPhase.CHECKING_SKIP_CONDITIONS: "checking skip conditions",
        ProgressPhase.EXECUTING: "executing",
        ProgressPhase.VERIFYING: "verifying",
    }[event.phase]
    if dry_run and event.phase is ProgressPhase.EXECUTING:
        phase_label = "executing (dry-run)"
    label = f"{event.step_name} [{event.step_id}]" if verbose else event.step_name
    return f"[{event.step_index}/{event.total_steps}] {label}: {phase_label}\n"


def _profile_match_facts(detected_device: DetectedDevice) -> ProfileMatchFacts:
    return ProfileMatchFacts(
        manufacturer=detected_device.manufacturer,
        brand=detected_device.brand or "",
        model=detected_device.model,
        android_version=detected_device.android_version,
    )


def _append_profile_mismatch_warning_to_draft(catalog, draft_plan: DraftPlan, detected_device: DetectedDevice | None) -> DraftPlan:
    warning = _build_profile_mismatch_warning(
        catalog=catalog,
        device_profile_ref=draft_plan.source.device_profile_ref,
        detected_device=detected_device,
    )
    if warning is None or any(existing.code is warning.code for existing in draft_plan.warnings):
        return draft_plan
    return replace(draft_plan, warnings=draft_plan.warnings + (warning,))


def _append_profile_mismatch_warning_to_result(
    catalog,
    result: PlanningResult,
    detected_device: DetectedDevice | None,
) -> PlanningResult:
    warning = None
    if result.execution_plan is not None:
        warning = _build_profile_mismatch_warning(
            catalog=catalog,
            device_profile_ref=result.execution_plan.source.device_profile_ref,
            detected_device=detected_device,
        )
    if warning is None or any(existing.code is warning.code for existing in result.warnings):
        return result
    status = result.status
    if status is PlanningStatus.SUCCESS:
        status = PlanningStatus.WARNING
    return replace(result, status=status, warnings=result.warnings + (warning,))


def _build_profile_mismatch_warning(catalog, device_profile_ref: str, detected_device: DetectedDevice | None) -> WarningMessage | None:
    if detected_device is None:
        return None
    profile = catalog.device_profiles[device_profile_ref]
    match = match_device_profile(profile, _profile_match_facts(detected_device))
    if match.matched:
        return None
    return WarningMessage(
        code=WarningCode.DEVICE_PROFILE_MISMATCH,
        message=(
            f"Selected device plan profile {device_profile_ref!r} does not match the connected device "
            f"{detected_device.manufacturer} {detected_device.model}."
        ),
        details={
            "device_profile_ref": device_profile_ref,
            "serial": detected_device.serial,
            "manufacturer": detected_device.manufacturer,
            "brand": detected_device.brand,
            "model": detected_device.model,
            "android_version": detected_device.android_version,
            "reasons": list(match.reasons),
        },
    )
