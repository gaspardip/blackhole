import { Hono } from "hono"; import {env} from "cloudflare:workers"; new Hono().request("/", {}, env);
