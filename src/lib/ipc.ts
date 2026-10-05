import { invoke, isTauri } from '@tauri-apps/api/core';
import type { AppError, DocumentWindow, FoundationHealth } from './types';

export function nativeAvailable(): boolean {
  return isTauri();
}

function structuredError(value: unknown): AppError | null {
  if (typeof value !== 'object' || value === null) return null;
  if (!('code' in value) || !('message' in value)) return null;
  if (typeof value.code !== 'string' || typeof value.message !== 'string') return null;
  return { code: value.code, message: value.message };
}

/** Unknown transport errors are never reflected as secret-bearing raw strings. */
export function toAppError(value: unknown): AppError {
  const error = structuredError(value);
  if (error) return error;
  if (typeof value === 'string') {
    try {
      const parsed: unknown = JSON.parse(value);
      const serializedError = structuredError(parsed);
      if (serializedError) return serializedError;
    } catch {
      // Non-structured transport text is intentionally not exposed.
    }
  }
  return { code: 'ipc', message: 'The native command could not be completed.' };
}

export class NativeCommandError extends Error implements AppError {
  readonly code: string;

  constructor(error: AppError) {
    super(error.message);
    this.name = 'NativeCommandError';
    this.code = error.code;
  }
}

/** No browser fallback: callers must visibly handle native_unavailable. */
export async function invokeCommand<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (!nativeAvailable()) {
    throw new NativeCommandError({
      code: 'native_unavailable',
      message: 'This action is available only in the native desktop application.',
    });
  }
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    throw new NativeCommandError(toAppError(error));
  }
}

export function getFoundationHealth(): Promise<FoundationHealth> {
  return invokeCommand<FoundationHealth>('foundation_health');
}

export function newDocumentWindow(): Promise<DocumentWindow> {
  return invokeCommand<DocumentWindow>('new_document_window');
}
