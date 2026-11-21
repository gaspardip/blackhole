import { Hono } from "hono"; function circular() { return circular().env; } new Hono().request("/", {}, circular());
