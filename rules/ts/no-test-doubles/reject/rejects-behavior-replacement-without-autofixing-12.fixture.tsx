const observer = vi.fn(); const { ["mockImplementation"]: replace } = observer; replace(() => null);
