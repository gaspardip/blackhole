import { Hono } from "hono"; const app = new Hono(); function bindings() { return {AUTH: {verify: () => true}}; } app.request("/", {}, bindings());
