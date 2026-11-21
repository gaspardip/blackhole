import { Hono } from "hono"; new Hono().request("/", {}, {API: {fetch: async () => new Response("fake")}});
