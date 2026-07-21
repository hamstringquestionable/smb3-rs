/* tslint:disable */
/* eslint-disable */

export function build_ips_patch(original: Uint8Array, modified: Uint8Array): Uint8Array;

export function decode_flag_key(key: string): string;

/**
 * Serialize the canonical Options::default() as JSON so the JS layer can
 * assert its schema covers every field (and only the fields) the Rust
 * source of truth knows about. Drift is reported on page load.
 */
export function default_options_json(): string;

export function encode_flag_key(options_json: string): string;

export function generate_patch(rom: Uint8Array, seed: bigint, options_json: string, visual_patch?: Uint8Array | null): Uint8Array;

export function generate_patched_rom(rom: Uint8Array, seed: bigint, options_json: string, visual_patch?: Uint8Array | null): Uint8Array;

/**
 * Validate that `rom` is a recognized SMB3 (USA) (Rev 1) dump. Intended to be
 * called from JS at upload time so the user sees errors immediately instead of
 * after clicking Generate. `skip_validation` mirrors the user-facing flag.
 */
export function validate_rom(rom: Uint8Array, skip_validation: boolean): void;

export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly build_ips_patch: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly decode_flag_key: (a: number, b: number) => [number, number, number, number];
    readonly default_options_json: () => [number, number, number, number];
    readonly encode_flag_key: (a: number, b: number) => [number, number, number, number];
    readonly generate_patch: (a: number, b: number, c: bigint, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly generate_patched_rom: (a: number, b: number, c: bigint, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly validate_rom: (a: number, b: number, c: number) => [number, number];
    readonly version: () => [number, number];
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
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
