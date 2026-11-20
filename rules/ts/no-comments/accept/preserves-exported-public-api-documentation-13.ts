interface Options { /** Retry limit. */ retries: number; }
type Settings = Options;
export const count = (options: Settings) => options.retries;
