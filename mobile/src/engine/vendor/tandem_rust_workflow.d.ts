// The wasm-bindgen package (engine/web/pkg, `--target web`) that mobile/scripts/build.mjs copies here.
import type { RustApp as App } from "../engine-rust.js";

export const RustApp: new () => App;
export default function init(options: { module_or_path: URL }): Promise<unknown>;
export function set_cjk_font(bytes: Uint8Array): boolean;
export function cjk_font_wanted(): boolean;
export function has_cjk_font(): boolean;
