import { expect, it } from "vitest";
import { notification, node } from "./dom";
import { button, Element, installDom } from "./test-dom";
installDom();
it("preserves the attached dismissal control for identical observations and resets only dismissible results", () => {
  const notice = notification("Result"); const root = notice.element as unknown as Element;
  notice.show("same", [node("p", "Done")]); const dismiss = button(root, "Dismiss result");
  notice.show("same", [node("p", "Done")]); expect(button(root, "Dismiss result")).toBe(dismiss); expect(dismiss.parent).toBe(root);
  dismiss.click(); notice.show("same", [node("p", "Done")]); expect(root.hidden).toBe(true);
  notice.show("same", [node("p", "Failed")], false); expect(root.hidden).toBe(false); expect(root.textContent).toContain("Failed");
});
