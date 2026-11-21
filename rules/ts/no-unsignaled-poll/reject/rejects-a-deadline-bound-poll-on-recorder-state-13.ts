await vi.waitFor(() => expect(cancel).not.toHaveBeenCalledWith(1));
