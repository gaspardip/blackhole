vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response());
