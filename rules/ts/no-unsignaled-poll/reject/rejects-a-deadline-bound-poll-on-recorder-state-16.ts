await vi.waitFor(() => expect(cancel.mock.calls.length).toBeGreaterThan(0));
