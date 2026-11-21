test("timer", async () => { vi["useFakeTimers"](); start(); await vi.advanceTimersByTimeAsync(1); expect(read()).toEqual(expected); });
