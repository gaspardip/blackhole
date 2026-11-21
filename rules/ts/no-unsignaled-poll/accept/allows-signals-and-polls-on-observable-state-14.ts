await vi.waitFor(() => { const count = requests.length; expect(count).toBe(1); });
