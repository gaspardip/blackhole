import Knock from "@knocklabs/node"; const observed = vi.spyOn(globalThis, "fetch"); new Knock({ fetch: observed });
