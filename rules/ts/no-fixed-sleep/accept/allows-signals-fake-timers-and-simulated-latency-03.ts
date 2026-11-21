test('x', async () => { vi.useFakeTimers(); start(); await vi.advanceTimersByTimeAsync(250); expect(readState()).toEqual(expectedState); });
