async function backoff(attempt) { await scheduler.wait(2 ** attempt); } test('x', async () => { await backoff(1); expect(readState()).toEqual(expectedState); });
