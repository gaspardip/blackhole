test("x", async () => { await new Promise(((resolve) => setTimeout(resolve, 1))); assert(work()); });
