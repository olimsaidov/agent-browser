import { hostStyle, markup } from "./cursor-artwork.js";

// Serialized into the controlled document. Keep this function self-contained:
// artwork and CDP coordinates are arguments, never ambient pointer events.
function paintCursor(mouse, hostStyle, markup) {
  const key = Symbol.for("agent-browser.wasm.cursor");
  let cursor = globalThis[key];
  if (!cursor?.host.isConnected) {
    const host = document.createElement("agent-browser-recording-cursor");
    host.setAttribute("data-agent-browser-recording-cursor", "");
    host.setAttribute("aria-hidden", "true");
    host.setAttribute("inert", "");
    host.style.cssText = hostStyle;
    const shadow = host.attachShadow({ mode: "closed" });
    shadow.innerHTML = markup;
    const pointer = shadow.querySelector(".pointer");
    document.documentElement.appendChild(host);
    cursor = globalThis[key] = { host, shadow, pointer };
  }

  const { pointer, shadow } = cursor;
  pointer.style.display = "block";
  pointer.style.transform = `translate3d(${mouse.x}px,${mouse.y}px,0)`;
  pointer.classList.toggle("pressed", mouse.buttons !== 0);
  if (mouse.type === "mousePressed") {
    const ripple = document.createElement("div");
    ripple.className = "ripple";
    ripple.style.left = `${mouse.x}px`;
    ripple.style.top = `${mouse.y}px`;
    ripple.addEventListener("animationend", () => ripple.remove(), { once: true });
    shadow.insertBefore(ripple, pointer);
  }
}

export function cursorExpression({ type, x, y, buttons = 0 }) {
  return `(${paintCursor.toString()})(${JSON.stringify({ type, x, y, buttons })},${JSON.stringify(hostStyle)},${JSON.stringify(markup)})`;
}
