import axios, { AxiosError } from "axios";

const API_TOKEN_STORAGE_KEY = "diving-api-token";

type AuthRequiredListener = (rejected: boolean) => void;
let authListener: AuthRequiredListener | null = null;

export const setAuthRequiredListener = (
  listener: AuthRequiredListener | null,
) => {
  authListener = listener;
};

export const getApiToken = (): string => {
  try {
    return sessionStorage.getItem(API_TOKEN_STORAGE_KEY) || "";
  } catch {
    return "";
  }
};

export const setApiToken = (token: string): string => {
  const trimmed = token.trim();
  try {
    if (trimmed) {
      sessionStorage.setItem(API_TOKEN_STORAGE_KEY, trimmed);
    } else {
      sessionStorage.removeItem(API_TOKEN_STORAGE_KEY);
    }
  } catch {
    /* ignore storage errors */
  }
  return trimmed;
};

export const clearApiToken = () => {
  try {
    sessionStorage.removeItem(API_TOKEN_STORAGE_KEY);
  } catch {
    /* ignore storage errors */
  }
};

export const request = axios.create({
  timeout: 600 * 1000,
  baseURL: "./api",
});

request.interceptors.request.use((config) => {
  const token = getApiToken();
  if (token) {
    config.headers = config.headers ?? {};
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

export const isApiAuthError = (err: unknown): boolean => {
  const axiosErr = err as AxiosError<{ category?: string }> | undefined;
  if (axiosErr?.response?.status !== 401) {
    return false;
  }
  const category = axiosErr.response.data?.category;
  return !category || category === "auth";
};

export const authFetch = async (
  input: string,
  init: RequestInit = {},
): Promise<Response> => {
  const token = getApiToken();
  const headers = new Headers(init.headers);
  if (token && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  const resp = await fetch(input, { ...init, headers });
  if (resp.status === 401) {
    let category: string | undefined;
    try {
      const body = (await resp.clone().json()) as { category?: string };
      category = body?.category;
    } catch {
      /* ignore non-JSON body */
    }
    if (!category || category === "auth") {
      authListener?.(Boolean(token));
    }
  }
  return resp;
};

export interface ParsedLatestImage {
  image: string;
  arch: string;
  href: string;
}

export const parseLatestImageRef = (
  raw: string,
  defaultArch = "amd64",
): ParsedLatestImage => {
  const qIdx = raw.indexOf("?");
  let image = raw;
  let arch = defaultArch;
  if (qIdx !== -1) {
    const base = raw.slice(0, qIdx);
    const params = new URLSearchParams(raw.slice(qIdx + 1));
    const parsedArch = params.get("arch")?.trim();
    if (parsedArch) {
      arch = parsedArch;
    }
    params.delete("arch");
    const rest = params.toString();
    image = rest ? `${base}?${rest}` : base;
  }
  const search = new URLSearchParams({ image, arch }).toString();
  return {
    image,
    arch,
    href: `./?${search}`,
  };
};
