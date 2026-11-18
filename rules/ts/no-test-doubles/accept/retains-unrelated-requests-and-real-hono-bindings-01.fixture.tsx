import { Hono } from "another-router"; new Hono().request("/", {}, {API: {get: () => null}});
