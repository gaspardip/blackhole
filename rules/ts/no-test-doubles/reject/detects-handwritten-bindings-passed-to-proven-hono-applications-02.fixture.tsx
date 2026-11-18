import { Hono as Router } from "hono"; const app = new Router().get("/", handler); const bindings = {TOKEN: {get: () => "secret"}}; app.request("/", {}, bindings);
