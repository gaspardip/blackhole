import { Hono } from "hono"; import {env} from "cloudflare:workers"; new Hono().request("/", {}, {EVENTS_SERVICE: env.EVENTS_SERVICE});
