import test from "node:test";
import assert from "node:assert/strict";
import { initialWorkflowState, workflowReducer, profileAuthorityReadIsCurrent, type WorkflowState } from "../src/workflow";
import type { DeviceFacts, DeviceMatch } from "../src/types";

test("recommendation cannot bypass native profile selection", () => {
  const selected = workflowReducer(initialWorkflowState, { type: "select-device", deviceHandle: "device" });
  const state = workflowReducer(selected, { type: "device-probed", facts: { deviceHandle: "device" } as DeviceFacts,
    match: { recommendedPlanId: "old-plan", candidates: [], safeGenericPlans: [], blocked: false } as unknown as DeviceMatch });
  assert.equal(state.step, "profile");
  assert.equal(state.devicePlan, null);
});

const profileMatch: DeviceMatch = { confidence:"exact", recommendedPlanId:"a.plan", requiresExplicitChoice:true,candidates:[],safeGenericPlans:[],blocked:false,blockReason:null,
  profileChangeAllowed:true, profiles:["a","b"].map((profileId) => ({profileId,profileName:profileId,profileDescription:null,generic:false,eligibility:"eligible",requiresExplicitChoice:true,reasons:[],plans:[{planId:profileId+".plan",profileId,profileName:profileId,name:profileId,description:null,reasons:[]}]})) };
const selection = (profileId:string, handle:string) => ({resolutionHandle:"next",selectionHandle:handle,selectedProfileId:profileId,setupGeneration:1,profileChangeAllowed:true,profileChangeReason:null});

test("profile switch clears setup and repair intent while keeping source saved values staged separately", () => {
  const before = { ...initialWorkflowState, deviceHandle:"device", facts:{deviceHandle:"device"} as DeviceFacts, match:profileMatch,
    selectionHandle:"old",selectedProfileId:"a", devicePlan:"a.plan", bindings:{theme:"dark"}, selectedRecipes:["recipe"],repairIntent:true,requiredReentryBindings:["token"],portableIntentDirty:true };
  const next=workflowReducer(before,{type:"profile-selected",selection:selection("b","new")});
  assert.equal(next.devicePlan,null); assert.deepEqual(next.bindings,{}); assert.deepEqual(next.requiredReentryBindings,[]); assert.equal(next.repairIntent,false); assert.equal(next.description,null); assert.equal(next.review,null); assert.ok(next.requestGeneration>before.requestGeneration);
});
test("same-profile saved intent requires native selection and changed compatibility baseline cannot activate", () => {
  for (const activationAllowed of [true,false]) {
    const source={theme:"dark"}; const bytes=JSON.stringify(source);
    let state=workflowReducer(initialWorkflowState,{type:"load-portable-intent",devicePlan:"a.plan",selectedRecipes:["recipe"],bindings:source,dirty:false,activationAllowed});
    assert.equal(state.devicePlan,null); assert.deepEqual(state.bindings,{});
    state={...state,deviceHandle:"device",match:profileMatch};
    const selected=workflowReducer(state,{type:"profile-selected",selection:selection("a","native")});
    assert.equal(selected.devicePlan,activationAllowed?"a.plan":null);
    assert.deepEqual(selected.bindings,activationAllowed?source:{}); assert.equal(JSON.stringify(source),bytes);
  }
});
test("unavailable execution blocks profile transitions and preserves its unresolved record", () => {
  const state={...initialWorkflowState,match:profileMatch,execution:{kind:"unavailable" as const,generation:1,executionHandle:"unresolved",message:"Unknown outcome",mode:"real" as const}};
  assert.equal(workflowReducer(state,{type:"choose-profile"}),state);
  assert.equal(workflowReducer(state,{type:"profile-selected",selection:selection("b","new")}),state);
});

test("a delayed authority error cannot clear a replacement selection at the same request generation", async () => {
  const captured = { runtimeGeneration: 3, requestGeneration: 7, selectionHandle: "selection-a" };
  let state: WorkflowState = { ...initialWorkflowState, step: "setup", deviceHandle: "device", match: profileMatch, selectionHandle: captured.selectionHandle, selectedProfileId: "a", requestGeneration: captured.requestGeneration, bindings: { theme: "current" } };
  let rejectRead: (reason: Error) => void = () => { throw new Error("read was not initialized"); };
  const pending = new Promise<never>((_resolve, reject) => { rejectRead = reject; });
  const handled = pending.catch(() => {
    if (!profileAuthorityReadIsCurrent({ runtimeGeneration: 3, requestGeneration: state.requestGeneration, selectionHandle: state.selectionHandle }, captured)) return;
    state = workflowReducer(state, { type: "profile-observed", selection: { resolutionHandle: null, selectionHandle: null, selectedProfileId: null, setupGeneration: 0, profileChangeAllowed: false, profileChangeReason: "Unavailable" } });
  });
  state = { ...state, selectionHandle: "selection-b", selectedProfileId: "b" };
  const replacement = state;
  rejectRead(new Error("older native read failed"));
  await handled;
  assert.equal(state, replacement);
  assert.equal(state.selectionHandle, "selection-b");
  assert.deepEqual(state.bindings, { theme: "current" });
});
