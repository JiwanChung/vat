/**
 * Type-safe HTTP client with automatic retry, circuit breaker,
 * and request/response interceptor pipeline.
 */

// --- Types ---

interface RequestConfig {
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  url: string;
  params?: Record<string, string | number | boolean>;
  body?: unknown;
  headers?: Record<string, string>;
  timeout?: number;
  signal?: AbortSignal;
  retry?: RetryConfig;
}

interface RetryConfig {
  maxRetries: number;
  baseDelay: number;
  maxDelay: number;
  retryOn: number[];
}

interface HttpResponse<T> {
  data: T;
  status: number;
  headers: Headers;
  duration: number;
  retries: number;
}

type Interceptor<T> = (value: T) => T | Promise<T>;

interface CircuitBreakerState {
  failures: number;
  lastFailure: number;
  state: "closed" | "open" | "half-open";
}

// --- Circuit Breaker ---

class CircuitBreaker {
  private circuits = new Map<string, CircuitBreakerState>();

  constructor(
    private threshold: number = 5,
    private resetTimeout: number = 30_000,
  ) {}

  getCircuitKey(url: string): string {
    try {
      const parsed = new URL(url);
      return parsed.origin;
    } catch {
      return url;
    }
  }

  canRequest(url: string): boolean {
    const key = this.getCircuitKey(url);
    const circuit = this.circuits.get(key);
    if (!circuit || circuit.state === "closed") return true;

    if (circuit.state === "open") {
      if (Date.now() - circuit.lastFailure > this.resetTimeout) {
        circuit.state = "half-open";
        return true;
      }
      return false;
    }

    return true; // half-open: allow one request through
  }

  recordSuccess(url: string): void {
    const key = this.getCircuitKey(url);
    this.circuits.set(key, { failures: 0, lastFailure: 0, state: "closed" });
  }

  recordFailure(url: string): void {
    const key = this.getCircuitKey(url);
    const circuit = this.circuits.get(key) ?? { failures: 0, lastFailure: 0, state: "closed" as const };
    circuit.failures++;
    circuit.lastFailure = Date.now();
    circuit.state = circuit.failures >= this.threshold ? "open" : "closed";
    this.circuits.set(key, circuit);
  }
}

// --- Client ---

const DEFAULT_RETRY: RetryConfig = {
  maxRetries: 3,
  baseDelay: 1000,
  maxDelay: 10_000,
  retryOn: [408, 429, 500, 502, 503, 504],
};

export class HttpClient {
  private baseUrl: string;
  private defaultHeaders: Record<string, string>;
  private defaultTimeout: number;
  private requestInterceptors: Interceptor<RequestConfig>[] = [];
  private responseInterceptors: Interceptor<HttpResponse<unknown>>[] = [];
  private breaker: CircuitBreaker;

  constructor(options: {
    baseUrl: string;
    headers?: Record<string, string>;
    timeout?: number;
    circuitBreaker?: { threshold?: number; resetTimeout?: number };
  }) {
    this.baseUrl = options.baseUrl.replace(/\/$/, "");
    this.defaultHeaders = {
      "Content-Type": "application/json",
      Accept: "application/json",
      ...options.headers,
    };
    this.defaultTimeout = options.timeout ?? 30_000;
    this.breaker = new CircuitBreaker(
      options.circuitBreaker?.threshold,
      options.circuitBreaker?.resetTimeout,
    );
  }

  onRequest(interceptor: Interceptor<RequestConfig>): this {
    this.requestInterceptors.push(interceptor);
    return this;
  }

  onResponse(interceptor: Interceptor<HttpResponse<unknown>>): this {
    this.responseInterceptors.push(interceptor);
    return this;
  }

  async get<T>(url: string, params?: Record<string, string | number | boolean>): Promise<HttpResponse<T>> {
    return this.request<T>({ method: "GET", url, params });
  }

  async post<T>(url: string, body?: unknown): Promise<HttpResponse<T>> {
    return this.request<T>({ method: "POST", url, body });
  }

  async put<T>(url: string, body?: unknown): Promise<HttpResponse<T>> {
    return this.request<T>({ method: "PUT", url, body });
  }

  async patch<T>(url: string, body?: unknown): Promise<HttpResponse<T>> {
    return this.request<T>({ method: "PATCH", url, body });
  }

  async delete<T>(url: string): Promise<HttpResponse<T>> {
    return this.request<T>({ method: "DELETE", url });
  }

  private async request<T>(config: RequestConfig): Promise<HttpResponse<T>> {
    let finalConfig = config;
    for (const interceptor of this.requestInterceptors) {
      finalConfig = await interceptor(finalConfig);
    }

    const fullUrl = this.buildUrl(finalConfig.url, finalConfig.params);
    const retryConfig = { ...DEFAULT_RETRY, ...finalConfig.retry };
    let lastError: Error | null = null;

    for (let attempt = 0; attempt <= retryConfig.maxRetries; attempt++) {
      if (!this.breaker.canRequest(fullUrl)) {
        throw new HttpError("Circuit breaker is open", 503, fullUrl);
      }

      try {
        const start = performance.now();
        const controller = new AbortController();
        const timeout = finalConfig.timeout ?? this.defaultTimeout;
        const timer = setTimeout(() => controller.abort(), timeout);

        const response = await fetch(fullUrl, {
          method: finalConfig.method,
          headers: { ...this.defaultHeaders, ...finalConfig.headers },
          body: finalConfig.body ? JSON.stringify(finalConfig.body) : undefined,
          signal: finalConfig.signal ?? controller.signal,
        });

        clearTimeout(timer);
        const duration = Math.round(performance.now() - start);

        if (!response.ok) {
          if (retryConfig.retryOn.includes(response.status) && attempt < retryConfig.maxRetries) {
            this.breaker.recordFailure(fullUrl);
            const delay = Math.min(
              retryConfig.baseDelay * Math.pow(2, attempt) + Math.random() * 1000,
              retryConfig.maxDelay,
            );
            await sleep(delay);
            continue;
          }

          this.breaker.recordFailure(fullUrl);
          const errorBody = await response.text().catch(() => "");
          throw new HttpError(
            `HTTP ${response.status}: ${response.statusText}`,
            response.status,
            fullUrl,
            errorBody,
          );
        }

        this.breaker.recordSuccess(fullUrl);
        const data = (await response.json()) as T;
        let result: HttpResponse<T> = { data, status: response.status, headers: response.headers, duration, retries: attempt };

        for (const interceptor of this.responseInterceptors) {
          result = (await interceptor(result as HttpResponse<unknown>)) as HttpResponse<T>;
        }

        return result;
      } catch (err) {
        if (err instanceof HttpError) throw err;
        lastError = err instanceof Error ? err : new Error(String(err));

        if (attempt < retryConfig.maxRetries) {
          this.breaker.recordFailure(fullUrl);
          await sleep(Math.min(retryConfig.baseDelay * Math.pow(2, attempt), retryConfig.maxDelay));
          continue;
        }
      }
    }

    throw lastError ?? new Error("Request failed after retries");
  }

  private buildUrl(path: string, params?: Record<string, string | number | boolean>): string {
    const url = path.startsWith("http") ? path : `${this.baseUrl}${path}`;
    if (!params || Object.keys(params).length === 0) return url;
    const searchParams = new URLSearchParams();
    for (const [key, value] of Object.entries(params)) {
      searchParams.set(key, String(value));
    }
    return `${url}?${searchParams.toString()}`;
  }
}

// --- Error Class ---

export class HttpError extends Error {
  constructor(
    message: string,
    public status: number,
    public url: string,
    public responseBody?: string,
  ) {
    super(message);
    this.name = "HttpError";
  }

  get isClientError(): boolean { return this.status >= 400 && this.status < 500; }
  get isServerError(): boolean { return this.status >= 500; }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// --- Factory ---

export function createApiClient(token: string) {
  const client = new HttpClient({
    baseUrl: "https://api.acme.dev/v2",
    headers: { Authorization: `Bearer ${token}` },
    timeout: 10_000,
    circuitBreaker: { threshold: 5, resetTimeout: 30_000 },
  });

  client
    .onRequest((config) => {
      config.headers = { ...config.headers, "X-Request-ID": crypto.randomUUID() };
      return config;
    })
    .onResponse((response) => {
      if (response.duration > 2000) {
        console.warn(`[slow] ${response.status} took ${response.duration}ms`);
      }
      return response;
    });

  return client;
}
