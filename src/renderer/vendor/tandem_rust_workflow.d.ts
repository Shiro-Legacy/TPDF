// The wasm-bindgen package (engine/web/pkg, `--target web`) that scripts/build.mjs copies here.
import type { RustApp as App } from "../engine-rust.js";

export const RustApp: new () => App;
export default function init(options: { module_or_path: URL }): Promise<unknown>;
