test("timer", async () => { vi["useFakeTimers"](); await scheduler.wait(1); expect(read()).toEqual(expected); });
