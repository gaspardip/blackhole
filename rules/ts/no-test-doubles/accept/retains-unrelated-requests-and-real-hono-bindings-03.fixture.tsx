import { Hono } from "hono"; new Hono().request("/", {}, {TOKEN: "value", FLAGS: {enabled: true}});
