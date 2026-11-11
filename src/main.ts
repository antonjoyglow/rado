import { getCurrentWindow } from "@tauri-apps/api/window";
import { mountList } from "./list";
import { mountPill } from "./pill";

const root = document.getElementById("app")!;
const label = getCurrentWindow().label;
document.body.dataset.view = label;

if (label === "pill") {
  mountPill(root);
} else {
  mountList(root);
}
