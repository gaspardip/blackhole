import { Hono } from "hono"; const handlers = {onEvent() {}}; new Hono().request("/", {}, handlers);
