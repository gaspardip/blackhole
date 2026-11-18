function guard(v: unknown): v is string { return true; }
function f(...items: string[]) { guard(items); }
