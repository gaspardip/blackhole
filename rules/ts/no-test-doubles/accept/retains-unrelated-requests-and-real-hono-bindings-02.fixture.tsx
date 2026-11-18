import { Hono } from "hono"; function run(Hono) { new Hono().request("/", {}, {API: {get: () => null}}); }
