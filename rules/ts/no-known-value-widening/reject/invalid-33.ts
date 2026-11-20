function isString(value: unknown): value is string { return true; } function check(known: string): boolean { return isString(known); }
