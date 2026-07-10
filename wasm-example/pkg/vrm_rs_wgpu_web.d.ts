/* tslint:disable */
/* eslint-disable */

export function orbit_wgpu_vrm_camera(delta_x: number, delta_y: number): void;

export function pan_wgpu_vrm_camera(delta_x: number, delta_y: number): void;

export function pointer_button_wgpu_vrm_ui(x: number, y: number, pressed: boolean): void;

export function pointer_leave_wgpu_vrm_ui(): void;

export function pointer_move_wgpu_vrm_ui(x: number, y: number): void;

export function reset_wgpu_vrm_camera(): void;

export function start_wgpu_vrm_viewer(canvas_id: string, avatar_bytes: Uint8Array, animation_a_bytes: Uint8Array, animation_b_bytes: Uint8Array): Promise<void>;

export function zoom_wgpu_vrm_camera(scroll_lines: number): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly orbit_wgpu_vrm_camera: (a: number, b: number) => [number, number];
    readonly pan_wgpu_vrm_camera: (a: number, b: number) => [number, number];
    readonly pointer_button_wgpu_vrm_ui: (a: number, b: number, c: number) => [number, number];
    readonly pointer_leave_wgpu_vrm_ui: () => [number, number];
    readonly pointer_move_wgpu_vrm_ui: (a: number, b: number) => [number, number];
    readonly reset_wgpu_vrm_camera: () => [number, number];
    readonly start_wgpu_vrm_viewer: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => any;
    readonly zoom_wgpu_vrm_camera: (a: number) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h8af5695b96ccf15b: (a: number, b: number, c: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h6c25144d5ae393cd: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h193aabdc3d7c65f4: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h7ec3ab38493a6e4b: (a: number, b: number, c: any) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
