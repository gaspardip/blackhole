import { Hono } from "hono"; import {FrontierAuth} from "../src/frontier-auth"; new Hono().request("/", {}, {AUTH_SERVICE: new FrontierAuth(state, env)});
