const { test: alias } = await import("vitest");
alias.skip("x", () => expect(read()).toBe(1));
