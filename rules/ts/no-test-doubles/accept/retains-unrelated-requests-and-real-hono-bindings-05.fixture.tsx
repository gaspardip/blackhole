import { Hono } from "hono"; import {Service} from "./service"; new Hono().request("/", {}, {API: new Service()});
