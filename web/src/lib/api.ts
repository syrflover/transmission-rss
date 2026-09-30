/**
 * The one way screens call the JSON API under `/api`.
 *
 * A failed call throws {@link ApiError} with the server's error shape
 * (`src/web/error.rs`): a code, a Korean sentence to show as is, and, on a
 * `conflict`, the server's current value when it sent one. A failure to reach
 * the server at all throws code `network` so the screen can say so and keep
 * the user's input.
 */

export type ApiErrorCode = "invalid" | "not_found" | "conflict" | "internal" | "network";

export class ApiError extends Error {
  readonly code: ApiErrorCode;
  readonly status: number;
  /** The server's current value on a `conflict`, for comparison with the input. */
  readonly current: unknown;

  constructor(code: ApiErrorCode, message: string, status: number, current?: unknown) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = status;
    this.current = current;
  }
}

const NETWORK_MESSAGE = "서버에 연결하지 못했어요. 연결을 확인하고 다시 시도해 주세요.";
const UNKNOWN_MESSAGE = "서버가 알 수 없는 응답을 보냈어요. 잠시 뒤 다시 시도해 주세요.";

/**
 * Sends a request to `/api{path}` and returns the parsed JSON body.
 * A `body` object is sent as JSON. A 204 response resolves to `undefined`.
 */
export async function api<T>(
  path: string,
  init: Omit<RequestInit, "body"> & { body?: unknown } = {},
): Promise<T> {
  const { body, headers, ...rest } = init;
  let response: Response;
  try {
    response = await fetch(`/api${path}`, {
      ...rest,
      headers: {
        Accept: "application/json",
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
        ...headers,
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError("network", NETWORK_MESSAGE, 0);
  }

  if (response.status === 204) return undefined as T;

  let json: unknown;
  try {
    json = await response.json();
  } catch {
    throw new ApiError("internal", UNKNOWN_MESSAGE, response.status);
  }

  if (!response.ok) {
    const error = json as { error?: ApiErrorCode; message?: string; current?: unknown };
    throw new ApiError(
      error.error ?? "internal",
      error.message ?? UNKNOWN_MESSAGE,
      response.status,
      error.current,
    );
  }
  return json as T;
}
