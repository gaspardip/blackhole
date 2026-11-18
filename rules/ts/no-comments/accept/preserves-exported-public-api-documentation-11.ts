interface Options { /** @deprecated Use attempts. */ retries: number; }
export function count(options: Options) { return options.retries; }
