/**
 * Structural validation for version 2 execution reports.
 *
 * This module reads the checked-in schema and validates report shape. The
 * qualification tool applies the record and eligibility bindings.
 */
import { readFileSync } from "node:fs";

const EXECUTION_REPORT_SCHEMA = JSON.parse(
  readFileSync(
    new URL("../docs/testing/device-qualification/execution-report.schema.json", import.meta.url),
    "utf8",
  ),
);
const RFC3339_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(Z|([+-])(\d{2}):(\d{2}))$/;

const SUPPORTED_EXECUTION_REPORT_SCHEMA_KEYWORDS = new Set([
  "$defs",
  "$id",
  "$ref",
  "$schema",
  "additionalProperties",
  "allOf",
  "anyOf",
  "const",
  "else",
  "enum",
  "format",
  "if",
  "items",
  "maximum",
  "minItems",
  "minLength",
  "minimum",
  "not",
  "oneOf",
  "pattern",
  "properties",
  "required",
  "then",
  "title",
  "type",
  "uniqueItems",
]);

function fail(message) {
  throw new Error(message);
}

export function assertSupportedExecutionReportSchema(schema) {
  const visit = (schemaNode, label) => {
    if (schemaNode === null || typeof schemaNode !== "object" || Array.isArray(schemaNode)) {
      fail(`${label} must be a schema object`);
    }
    for (const keyword of Object.keys(schemaNode)) {
      if (!SUPPORTED_EXECUTION_REPORT_SCHEMA_KEYWORDS.has(keyword)) {
        fail(`unsupported execution report schema keyword ${keyword} at ${label}`);
      }
    }
    for (const [name, child] of Object.entries(schemaNode.$defs ?? {})) {
      visit(child, `${label}.$defs.${name}`);
    }
    for (const [name, child] of Object.entries(schemaNode.properties ?? {})) {
      visit(child, `${label}.properties.${name}`);
    }
    if (schemaNode.items !== undefined) {
      if (Array.isArray(schemaNode.items)) {
        schemaNode.items.forEach((child, index) =>
          visit(child, `${label}.items[${index}]`),
        );
      } else {
        visit(schemaNode.items, `${label}.items`);
      }
    }
    for (const keyword of ["oneOf", "anyOf", "allOf"]) {
      schemaNode[keyword]?.forEach((child, index) =>
        visit(child, `${label}.${keyword}[${index}]`),
      );
    }
    for (const keyword of ["if", "then", "else", "not"]) {
      if (schemaNode[keyword] !== undefined) {
        visit(schemaNode[keyword], `${label}.${keyword}`);
      }
    }
  };

  visit(schema, "execution report schema");
}

assertSupportedExecutionReportSchema(EXECUTION_REPORT_SCHEMA);

function assertObject(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(label + " must be an object");
  }
}

function assertExactKeys(value, expected, label) {
  assertObject(value, label);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(label + " fields must be exactly " + wanted.join(", "));
  }
}

function assertString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    fail(label + " must be a non-empty string");
  }
}

function assertBoolean(value, label) {
  if (typeof value !== "boolean") {
    fail(label + " must be a boolean");
  }
}

function assertStringArray(value, label) {
  if (
    !Array.isArray(value) ||
    value.some((item) => typeof item !== "string" || item.length === 0)
  ) {
    fail(label + " must be an array of non-empty strings");
  }
}

function assertPositiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(label + " must be a positive integer");
  }
}

function assertNonNegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    fail(label + " must be a non-negative integer");
  }
}

function assertRfc3339(value, label) {
  assertString(value, label);
  const match = RFC3339_PATTERN.exec(value);
  if (match === null || Number.isNaN(Date.parse(value))) {
    fail(label + " must be an RFC3339 timestamp");
  }
  const [
    ,
    yearText,
    monthText,
    dayText,
    hourText,
    minuteText,
    secondText,
    ,
    ,
    offsetHourText,
    offsetMinuteText,
  ] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  const offsetHour = offsetHourText === undefined ? 0 : Number(offsetHourText);
  const offsetMinute = offsetMinuteText === undefined ? 0 : Number(offsetMinuteText);
  const isLeapYear = year % 400 === 0 || (year % 4 === 0 && year % 100 !== 0);
  const monthDays = [31, isLeapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > monthDays[month - 1] ||
    hour > 23 ||
    minute > 59 ||
    second > 59 ||
    offsetHour > 23 ||
    offsetMinute > 59
  ) {
    fail(label + " must be an RFC3339 timestamp");
  }
}

function resolveExecutionReportSchema(schema) {
  if (!schema?.$ref) return schema;
  const prefix = "#/$defs/";
  if (!schema.$ref.startsWith(prefix)) {
    fail(`unsupported execution report schema reference ${schema.$ref}`);
  }
  const definition = EXECUTION_REPORT_SCHEMA.$defs[schema.$ref.slice(prefix.length)];
  if (definition === undefined) {
    fail(`missing execution report schema definition ${schema.$ref}`);
  }
  return definition;
}

function assertExecutionReportObject(value, schema, label) {
  assertObject(value, label);
  const definition = resolveExecutionReportSchema(schema);
  if (definition.type !== "object") {
    fail(`${label} schema must describe an object`);
  }
  const required = definition.required ?? [];
  const properties = definition.properties ?? {};
  const expected = [
    ...required,
    ...Object.keys(properties).filter(
      (field) => !required.includes(field) && Object.hasOwn(value, field),
    ),
  ];
  assertExactKeys(value, expected, label);
}

function assertExecutionReportSchemaEnum(value, schema, label) {
  const definition = resolveExecutionReportSchema(schema);
  const allowed = definition.enum ??
    (Object.hasOwn(definition, "const") ? [definition.const] : null);
  if (allowed === null || !allowed.includes(value)) {
    fail(`${label} value ${String(value)} is not supported`);
  }
}

function assertExecutionReportSchemaPattern(value, schema, label) {
  const definition = resolveExecutionReportSchema(schema);
  if (typeof value !== "string" || !new RegExp(definition.pattern).test(value)) {
    fail(`${label} does not match its schema pattern`);
  }
}

function validateExecutionReportBuildIdentity(buildIdentity, label) {
  const schema = EXECUTION_REPORT_SCHEMA.$defs.buildIdentity;
  assertExecutionReportObject(buildIdentity, schema, label);
  assertString(buildIdentity.appVersion, `${label} appVersion`);
  assertExecutionReportSchemaPattern(
    buildIdentity.gitCommit,
    schema.properties.gitCommit,
    `${label} gitCommit`,
  );
  assertExecutionReportSchemaPattern(
    buildIdentity.materialBuildDigest,
    schema.properties.materialBuildDigest,
    `${label} materialBuildDigest`,
  );
  assertBoolean(buildIdentity.realExecutionEnabled, `${label} realExecutionEnabled`);
  assertPositiveInteger(
    buildIdentity.qualificationContract,
    `${label} qualificationContract`,
  );
  return buildIdentity;
}

function assertExecutionReportNullableString(value, label) {
  if (value !== null && typeof value !== "string") {
    fail(`${label} must be a string or null`);
  }
}

function assertExecutionReportTimestamp(value, label) {
  if (value !== null) assertRfc3339(value, label);
}

// Canonical evidence accepts only digest-bound v2 execution reports.
// Restricted claim bindings use the same validated report shape.
function validateExecutionReportDocument(report) {
  assertObject(report, "execution report");
  assertExecutionReportSchemaEnum(
    report.schema,
    EXECUTION_REPORT_SCHEMA.properties.schema,
    "execution report schema",
  );
  const requiredVersion = EXECUTION_REPORT_SCHEMA.properties.schemaVersion.const;
  if (report.schemaVersion !== requiredVersion) {
    fail(`execution report schemaVersion must be ${requiredVersion}`);
  }
  const hasQualification = Object.hasOwn(report, "qualification");
  assertExecutionReportObject(
    report,
    EXECUTION_REPORT_SCHEMA,
    "execution report",
  );

  assertExecutionReportObject(
    report.app,
    EXECUTION_REPORT_SCHEMA.$defs.app,
    "execution report app",
  );
  assertString(report.app.name, "execution report app name");
  assertString(report.app.version, "execution report app version");

  validateExecutionReportRuntime(report.runtime);
  validateExecutionReportCatalog(report.catalog);
  validateExecutionReportPlan(report.plan);
  validateExecutionReportExecution(report.execution, hasQualification);
  if (hasQualification) {
    validateExecutionReportQualification(report.qualification);
  }
  return report;
}

function validateExecutionReportQualification(qualification) {
  const schema = EXECUTION_REPORT_SCHEMA.$defs.qualification;
  assertExecutionReportObject(
    qualification,
    schema,
    "execution report qualification",
  );
  for (const field of [
    "claim",
    "executionMode",
    "strategy",
    "restriction",
    "observedRootState",
  ]) {
    assertExecutionReportSchemaEnum(
      qualification[field],
      schema.properties[field],
      "execution report qualification " + field,
    );
  }
  assertString(qualification.attemptId, "restricted qualification attemptId");

  if (!Array.isArray(qualification.effectiveCapabilities)) {
    fail("restricted qualification effectiveCapabilities must be an array");
  }
  const capabilities = new Set();
  for (const capability of qualification.effectiveCapabilities) {
    assertString(capability, "restricted qualification capability");
    if (capabilities.has(capability)) {
      fail("duplicate restricted qualification capability " + capability);
    }
    capabilities.add(capability);
  }

  validateExecutionReportBuildIdentity(
    qualification.buildIdentity,
    "restricted qualification buildIdentity",
  );
  for (const field of [
    "deviceTargetId",
    "deviceProfileId",
    "reviewedPlanId",
  ]) {
    assertString(
      qualification[field],
      "restricted qualification " + field,
    );
  }
  assertExecutionReportSchemaPattern(
    qualification.reviewedPlanDigest,
    schema.properties.reviewedPlanDigest,
    "restricted qualification reviewedPlanDigest",
  );

  const requiredOperationIds = qualification.requiredOperationIds;
  if (!Array.isArray(requiredOperationIds) || requiredOperationIds.length === 0) {
    fail("restricted qualification requiredOperationIds must be a non-empty array");
  }
  const requiredOperationIdsSeen = new Set();
  for (const operationId of requiredOperationIds) {
    assertString(operationId, "restricted qualification required operation id");
    if (requiredOperationIdsSeen.has(operationId)) {
      fail("duplicate restricted qualification required operation " + operationId);
    }
    requiredOperationIdsSeen.add(operationId);
  }

  if (!Array.isArray(qualification.operationOutcomes)) {
    fail("restricted qualification operationOutcomes must be an array");
  }
  const outcomeSchema = EXECUTION_REPORT_SCHEMA.$defs.operationOutcome;
  for (const outcome of qualification.operationOutcomes) {
    assertExecutionReportObject(
      outcome,
      outcomeSchema,
      "restricted qualification operation outcome",
    );
    assertString(outcome.operationId, "restricted qualification operationId");
    assertExecutionReportSchemaEnum(
      outcome.outcome,
      outcomeSchema.properties.outcome,
      "restricted qualification operation outcome",
    );
  }
}

function validateExecutionReportRuntime(runtime) {
  assertObject(runtime, "execution report runtime");
  const hasRestriction = Object.hasOwn(runtime, "executionRestriction");
  const runtimeVariant = EXECUTION_REPORT_SCHEMA.$defs.runtime.oneOf.find((variant) => {
    const status = variant.properties.status;
    return status.const === runtime.status || status.enum?.includes(runtime.status);
  });
  if (runtimeVariant === undefined) {
    fail("execution report runtime status is not supported");
  }
  assertExecutionReportObject(runtime, runtimeVariant, "execution report runtime");
  switch (runtime.status) {
    case "starting":
      break;
    case "ready":
      assertPositiveInteger(
        runtime.protocolVersion,
        "execution report runtime protocolVersion",
      );
      assertExecutionReportNullableString(
        runtime.catalogVersion,
        "execution report runtime catalogVersion",
      );
      if (runtime.catalogVersion !== null) {
        assertString(runtime.catalogVersion, "execution report runtime catalogVersion");
      }
      break;
    case "unsupported":
    case "failed":
      assertExecutionReportObject(
        runtime.error,
        EXECUTION_REPORT_SCHEMA.$defs.runtimeError,
        "execution report runtime error",
      );
      assertString(runtime.error.code, "execution report runtime error code");
      assertString(runtime.error.message, "execution report runtime error message");
      assertStringArray(runtime.error.actions, "execution report runtime error actions");
      break;
  }

  if (hasRestriction) {
    const restriction = runtime.executionRestriction;
    assertExecutionReportObject(
      restriction,
      EXECUTION_REPORT_SCHEMA.$defs.executionRestriction,
      "execution report runtime executionRestriction",
    );
    assertBoolean(
      restriction.enabled,
      "execution report runtime executionRestriction enabled",
    );
    validateExecutionReportBuildIdentity(
      restriction.buildIdentity,
      "execution report runtime executionRestriction buildIdentity",
    );
    assertPositiveInteger(
      restriction.runtimeGeneration,
      "execution report runtime executionRestriction runtimeGeneration",
    );
    assertString(
      restriction.strategy,
      "execution report runtime executionRestriction strategy",
    );
    assertBoolean(
      restriction.rootShell,
      "execution report runtime executionRestriction rootShell",
    );
    assertBoolean(
      restriction.appDataWrite,
      "execution report runtime executionRestriction appDataWrite",
    );
  }
}

function validateExecutionReportCatalog(catalog) {
  if (catalog === null) return;
  const schema = EXECUTION_REPORT_SCHEMA.$defs.catalog;
  assertExecutionReportObject(
    catalog,
    schema,
    "execution report catalog",
  );
  assertExecutionReportSchemaEnum(
    catalog.sourceKind,
    schema.properties.sourceKind,
    "execution report catalog sourceKind",
  );
  assertString(catalog.sourceId, "execution report catalog sourceId");
  assertExecutionReportNullableString(catalog.version, "execution report catalog version");
  if (catalog.version !== null) {
    assertString(catalog.version, "execution report catalog version");
  }
  assertExecutionReportNullableString(catalog.cacheKey, "execution report catalog cacheKey");
  if (catalog.cacheKey !== null) {
    assertString(catalog.cacheKey, "execution report catalog cacheKey");
  }
  if (catalog.contentDigest === null) return;
  const digestSchema = schema.properties.contentDigest.oneOf.find(
    (candidate) => candidate.type === "object",
  );
  assertExecutionReportObject(
    catalog.contentDigest,
    digestSchema,
    "execution report catalog contentDigest",
  );
  assertExecutionReportSchemaEnum(
    catalog.contentDigest.algorithm,
    digestSchema.properties.algorithm,
    "execution report catalog contentDigest algorithm",
  );
  assertExecutionReportSchemaPattern(
    catalog.contentDigest.value,
    digestSchema.properties.value,
    "execution report catalog contentDigest value",
  );
}

function validateExecutionReportPlan(plan) {
  const schema = EXECUTION_REPORT_SCHEMA.$defs.plan;
  assertExecutionReportObject(plan, schema, "execution report plan");
  assertExecutionReportNullableString(plan.planId, "execution report plan planId");
  if (plan.planId !== null) assertString(plan.planId, "execution report plan planId");
  if (plan.planDigest !== null) {
    assertString(plan.planDigest, "execution report plan planDigest");
    const digestPattern = schema.properties.planDigest.oneOf.find(
      (candidate) => candidate.type === "string",
    );
    assertExecutionReportSchemaPattern(
      plan.planDigest,
      digestPattern,
      "execution report plan planDigest",
    );
  }
}

function validateExecutionReportCompletion(completion, warnings) {
  const schema = EXECUTION_REPORT_SCHEMA.$defs.completion;
  assertExecutionReportObject(
    completion,
    schema,
    "execution report completion",
  );
  assertExecutionReportSchemaEnum(
    completion.classification,
    schema.properties.classification,
    "execution report completion classification",
  );
  const countSchema = EXECUTION_REPORT_SCHEMA.$defs.counts;
  const countFields = Object.keys(countSchema.properties);
  assertExecutionReportObject(completion.counts, countSchema, "execution report completion counts");
  for (const field of countFields) {
    assertNonNegativeInteger(
      completion.counts[field],
      `execution report completion counts ${field}`,
    );
  }
  assertNonNegativeInteger(completion.warningCount, "execution report completion warningCount");
  if (completion.warningCount !== warnings.length) {
    fail("execution report completion warningCount must match warnings");
  }
  assertBoolean(
    completion.partialChangesPossible,
    "execution report completion partialChangesPossible",
  );
  if (!Array.isArray(completion.features)) {
    fail("execution report completion features must be an array");
  }
  const featureSchema = EXECUTION_REPORT_SCHEMA.$defs.featureCompletion;
  const featureCountSchema = EXECUTION_REPORT_SCHEMA.$defs.featureCounts;
  for (const feature of completion.features) {
    assertExecutionReportObject(
      feature,
      featureSchema,
      "execution report completion feature",
    );
    assertString(feature.name, "execution report completion feature name");
    assertExecutionReportSchemaEnum(
      feature.status,
      featureSchema.properties.status,
      "execution report completion feature status",
    );
    assertExecutionReportObject(
      feature.counts,
      featureCountSchema,
      "execution report completion feature counts",
    );
    for (const [field, value] of Object.entries(feature.counts)) {
      assertNonNegativeInteger(value, `execution report completion feature counts ${field}`);
    }
  }
}

function validateExecutionReportIssue(issue, label) {
  assertExecutionReportObject(issue, EXECUTION_REPORT_SCHEMA.$defs.issue, label);
  assertString(issue.message, `${label} message`);
  const remediationSchema = EXECUTION_REPORT_SCHEMA.$defs.remediation;
  assertExecutionReportObject(
    issue.remediation,
    remediationSchema,
    `${label} remediation`,
  );
  assertExecutionReportSchemaEnum(
    issue.remediation.kind,
    remediationSchema.properties.kind,
    `${label} remediation kind`,
  );
  assertString(issue.remediation.title, `${label} remediation title`);
  assertString(issue.remediation.message, `${label} remediation message`);
}

function validateExecutionReportTarget(target, simulated) {
  if (target === null) {
    if (!simulated) fail("real execution report target must be an object");
    return;
  }
  if (simulated) fail("simulated execution report target must be null");
  const schema = EXECUTION_REPORT_SCHEMA.$defs.target;
  assertExecutionReportObject(
    target,
    schema,
    "execution report target",
  );
  assertString(target.label, "execution report target label");
  for (const field of ["manufacturer", "model", "androidVersion"]) {
    if (Object.hasOwn(target, field)) {
      assertString(target[field], `execution report target ${field}`);
    }
  }
  if (Object.hasOwn(target, "androidApiLevel")) {
    assertPositiveInteger(target.androidApiLevel, "execution report target androidApiLevel");
  }
}

function validateExecutionReportRelease(release) {
  const schema = EXECUTION_REPORT_SCHEMA.$defs.resolvedRelease;
  assertExecutionReportObject(
    release,
    schema,
    "execution report resolved release",
  );
  for (const field of ["recipeArtifactId", "appId", "artifactId", "releaseTag", "assetName"]) {
    assertString(release[field], `execution report resolved release ${field}`);
  }
  assertExecutionReportSchemaEnum(
    release.provider,
    schema.properties.provider,
    "execution report resolved release provider",
  );
  assertExecutionReportSchemaEnum(
    release.kind,
    schema.properties.kind,
    "execution report resolved release kind",
  );
  assertExecutionReportTimestamp(
    release.publishedAt,
    "execution report resolved release publishedAt",
  );
  assertExecutionReportTimestamp(
    release.createdAt,
    "execution report resolved release createdAt",
  );
  assertExecutionReportSchemaEnum(
    release.timestampSource,
    schema.properties.timestampSource,
    "execution report resolved release timestampSource",
  );
  if (release.size !== null) {
    assertNonNegativeInteger(release.size, "execution report resolved release size");
  }
  assertString(release.calculatedSha256, "execution report resolved release calculatedSha256");
  assertExecutionReportSchemaPattern(
    release.calculatedSha256,
    schema.properties.calculatedSha256,
    "execution report resolved release calculatedSha256",
  );
  assertBoolean(release.cacheHit, "execution report resolved release cacheHit");
  assertExecutionReportNullableString(
    release.redactedFinalUrl,
    "execution report resolved release redactedFinalUrl",
  );
  if (release.redactedFinalUrl !== null) {
    let parsed;
    try {
      parsed = new URL(release.redactedFinalUrl);
    } catch {
      fail("execution report resolved release redactedFinalUrl must be a valid URL");
    }
    if (
      parsed.protocol !== "https:" ||
      parsed.username !== "" ||
      parsed.password !== "" ||
      parsed.search !== "" ||
      parsed.hash !== ""
    ) {
      fail("execution report resolved release redactedFinalUrl contains unredacted URL data");
    }
  }
}

function validateExecutionReportExecution(execution, hasQualification) {
  const schema = EXECUTION_REPORT_SCHEMA.$defs.execution;
  assertExecutionReportObject(
    execution,
    schema,
    "execution report execution",
  );
  assertBoolean(execution.simulated, "execution report execution simulated");
  const expectedScope = execution.simulated ? "simulation_only" : "real_device";
  if (execution.verificationScope !== expectedScope) {
    fail("execution report verificationScope must match simulated state");
  }
  assertExecutionReportSchemaEnum(
    execution.status,
    schema.properties.status,
    "execution report execution status",
  );
  assertExecutionReportTimestamp(execution.startedAt, "execution report execution startedAt");
  assertExecutionReportTimestamp(execution.finishedAt, "execution report execution finishedAt");
  if (!Array.isArray(execution.warnings) || !Array.isArray(execution.errors)) {
    fail("execution report warnings and errors must be arrays");
  }
  execution.warnings.forEach((issue, index) =>
    validateExecutionReportIssue(issue, `execution report warning ${index}`),
  );
  execution.errors.forEach((issue, index) =>
    validateExecutionReportIssue(issue, `execution report error ${index}`),
  );
  validateExecutionReportCompletion(execution.completion, execution.warnings);

  if (!Array.isArray(execution.recipes)) {
    fail("execution report recipes must be an array");
  }
  const recipeIds = new Set();
  const stepIds = new Set();
  const recipeSchema = EXECUTION_REPORT_SCHEMA.$defs.recipe;
  const stepSchema = EXECUTION_REPORT_SCHEMA.$defs.step;
  for (const recipe of execution.recipes) {
    assertExecutionReportObject(recipe, recipeSchema, "execution report recipe");
    if (hasQualification) {
      assertString(recipe.recipeId, "execution report recipe recipeId");
      if (recipeIds.has(recipe.recipeId)) {
        fail(`execution report recipe ID ${recipe.recipeId} is ambiguous`);
      }
      recipeIds.add(recipe.recipeId);
    } else {
      if (Object.hasOwn(recipe, "recipeId")) {
        fail("ordinary execution report must omit recipe and step IDs");
      }
    }
    assertString(recipe.name, "execution report recipe name");
    assertExecutionReportNullableString(recipe.description, "execution report recipe description");
    assertExecutionReportSchemaEnum(
      recipe.status,
      recipeSchema.properties.status,
      "execution report recipe status",
    );
    if (!Array.isArray(recipe.steps)) {
      fail("execution report recipe steps must be an array");
    }
    for (const step of recipe.steps) {
      assertExecutionReportObject(step, stepSchema, "execution report step");
      if (hasQualification) {
        assertString(step.recipeId, "execution report step recipeId");
        assertString(step.stepId, "execution report step stepId");
        if (step.recipeId !== recipe.recipeId) {
          fail("execution report step recipeId must match its containing recipe");
        }
        const stepKey = `${step.recipeId}\u0000${step.stepId}`;
        if (stepIds.has(stepKey)) {
          fail(`execution report step ID ${step.stepId} is ambiguous`);
        }
        stepIds.add(stepKey);
      } else {
        if (Object.hasOwn(step, "recipeId") || Object.hasOwn(step, "stepId")) {
          fail("ordinary execution report must omit recipe and step IDs");
        }
      }
      assertString(step.name, "execution report step name");
      assertExecutionReportNullableString(step.note, "execution report step note");
      assertExecutionReportSchemaEnum(
        step.status,
        stepSchema.properties.status,
        "execution report step status",
      );
      assertExecutionReportNullableString(step.message, "execution report step message");
    }
  }

  validateExecutionReportTarget(execution.target, execution.simulated);
  if (!Array.isArray(execution.resolvedReleases)) {
    fail("execution report resolvedReleases must be an array");
  }
  execution.resolvedReleases.forEach(validateExecutionReportRelease);

  const expectedClassification = {
    succeeded: "success",
    succeeded_with_warnings: "success_with_warnings",
    failed: "failed",
    cancelled: "cancelled",
  }[execution.status];
  if (execution.completion.classification !== expectedClassification) {
    fail("execution report completion classification must match execution status");
  }
}

function parseExecutionReportDocument(reportBytes) {
  let report;
  try {
    report = JSON.parse(reportBytes.toString("utf8"));
  } catch {
    fail("execution report JSON is invalid");
  }
  return report;
}

function isHistoricalV1ExecutionReport(report) {
  return (
    report !== null &&
    typeof report === "object" &&
    !Array.isArray(report) &&
    report.schema === EXECUTION_REPORT_SCHEMA.properties.schema.const &&
    report.schemaVersion === 1
  );
}

/** Validate one v2 report, with optional v1 inventory classification. */
export function validateExecutionReportForInventory(
  reportBytes,
  { allowHistoricalV1 = false } = {},
) {
  const report = parseExecutionReportDocument(reportBytes);
  if (allowHistoricalV1 && isHistoricalV1ExecutionReport(report)) return null;
  return validateExecutionReportDocument(report);
}

/** Parse and validate one complete v2 execution report. */
export function validateExecutionReport(reportBytes) {
  return validateExecutionReportForInventory(reportBytes);
}
