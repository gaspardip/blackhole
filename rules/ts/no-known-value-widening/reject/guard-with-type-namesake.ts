type guard = string; function guard(v: unknown): v is string { return true; } guard("x");
