/** Aster Companion API v1; no bundled Cubism runtime or model assets. */
export type State = 'idle' | 'listening' | 'thinking' | 'reading' | 'working' | 'checking' | 'waiting' | 'speaking' | 'pleased' | 'concerned';
export interface Profile {
  version: 1;
  name: string;
  /** Relative to --pet-dir; the model's own references stay local. */
  model: string;
  layout: { zoom: number; center: [number, number]; anchor: [number, number] };
  bindings: Record<string, string>;
  emotions: Record<string, Record<string, number>>;
  motions: Record<string, { duration_ms: number; blend: 'add' | 'set'; tracks: Record<string, [number, number][]> }>;
}
export type Control =
  | { type: 'emotion'; name: string; strength: number }
  | { type: 'motion'; name: string; strength: number }
  | { type: 'look'; x: number; y: number; duration_ms: number }
  | { type: 'reset' };
export interface Parameter { id: string; min: number; max: number; default: number }
export interface Description {
  version: 1; profile: string; model: string; states: State[];
  emotions: string[]; motions: string[]; bindings: Record<string, string>;
  parameters: Parameter[]; warnings: string[];
}
export interface Snapshot {
  version: 1; state: State; emotion: string; strength: number;
  motion: string | null; look: [number, number] | null;
  time: number; parameters: Record<string, number>;
}
export interface ControlResult {
  emotion?: string; motion?: string; strength?: number; look?: [number, number];
  duration_ms?: number; supported_channels?: string[]; reset?: true;
}
/** The narrow adapter boundary used from pixi-live2d-display's Cubism4 coreModel. */
export interface CoreModel {
  getModel(): { parameters: { ids: ArrayLike<string>; minimumValues: ArrayLike<number>; maximumValues: ArrayLike<number>; defaultValues: ArrayLike<number> } };
  setParameterValueByIndex(index: number, value: number): void;
}
export interface Controller {
  describe(): Description;
  state(value: State): void;
  emotion(name: string, strength?: number): ControlResult;
  motion(name: string, strength?: number): ControlResult;
  look(x: number, y: number, duration_ms?: number): ControlResult;
  reset(): ControlResult;
  dispatch(command: Control): ControlResult;
  /** Milliseconds, 0–250. Call exactly once per rendered frame, after physics. */
  apply(delta_ms?: number): Record<string, number>;
  snapshot(): Snapshot;
}
export const version: 1;
export function validateProfile(profile: unknown): Profile;
export function create(core: CoreModel, profile: Profile): Controller;
export as namespace AsterCompanion;
declare global { interface Window { asterCompanion?: Controller } }
