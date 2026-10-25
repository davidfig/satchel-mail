// Mirrors crates/api/src/lib.rs. Keep in sync when the schema changes.

export type Id = string;
export type ISODate = string;

export interface Address {
  name?: string | null;
  email: string;
}
