await vi.waitFor(() => expect(count()).toBe(1), { timeout: 5_000 });
