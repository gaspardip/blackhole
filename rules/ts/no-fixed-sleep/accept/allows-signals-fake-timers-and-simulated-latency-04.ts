test('x', async () => { serve(async () => { await scheduler.wait(10); return new Response(); }); expect(readState()).toEqual(expectedState); });
