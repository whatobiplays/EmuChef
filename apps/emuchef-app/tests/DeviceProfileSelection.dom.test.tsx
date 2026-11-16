import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { DeviceProfileSelection } from "../src/DeviceProfileSelection";
import type { DeviceFacts, DeviceMatch } from "../src/types";
const facts = { manufacturer:"ARBOR", model:"GT78-VN", androidVersion:12 } as DeviceFacts;
const match: DeviceMatch = { confidence:"exact", recommendedPlanId:null, requiresExplicitChoice:true, candidates:[], safeGenericPlans:[], blocked:false, blockReason:null,
  resolutionHandle:"resolution", profileChangeAllowed:true, resolutionStatus:"choice_required", profiles:["KONKR Pocket Advance","AYANEO Pocket Air Mini"].map((profileName,index) => ({profileId:String(index), profileName, profileDescription:null, generic:false, eligibility:"eligible",requiresExplicitChoice:true,reasons:["Model matched"],plans:[]})) };
test("Tab only focuses and Enter confirms a dynamic candidate once", async () => {
  const user=userEvent.setup(); const onSelect=vi.fn(); render(<DeviceProfileSelection match={match} facts={facts} busy={false} onSelect={onSelect} />);
  await user.tab(); expect(document.activeElement).toBe(screen.getByRole("button",{name:"KONKR Pocket Advance"})); expect(onSelect).not.toHaveBeenCalled();
  await user.keyboard("{Enter}"); expect(onSelect).toHaveBeenCalledExactlyOnceWith("0",false);
});
test("Space confirms a candidate without an extra dialog", async () => {
  const user=userEvent.setup(); const onSelect=vi.fn(); render(<DeviceProfileSelection match={match} facts={facts} busy={false} onSelect={onSelect} />);
  screen.getByRole("button",{name:"AYANEO Pocket Air Mini"}).focus(); await user.keyboard(" "); expect(onSelect).toHaveBeenCalledExactlyOnceWith("1",false); expect(screen.queryByRole("dialog")).toBeNull();
});
test("session invalidation is explained and unavailable authority disables choice", () => {
  render(<DeviceProfileSelection match={{...match,profileChangeAllowed:false,profileSelectionNotice:"The device reconnected. Choose its Device Profile again."}} facts={facts} busy={false} onSelect={vi.fn()} />);
  expect(screen.getByText("The device reconnected. Choose its Device Profile again.")).toBeTruthy();
  expect((screen.getByRole("button",{name:"KONKR Pocket Advance"}) as HTMLButtonElement).disabled).toBe(true);
});
