test("x", (async () => { await scheduler.wait(1); assert(work()); }));
