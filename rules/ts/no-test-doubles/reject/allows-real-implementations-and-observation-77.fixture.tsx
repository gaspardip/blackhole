function instrument(Reflect) { Reflect.apply(vi.mock, vi, ["client"]); }
