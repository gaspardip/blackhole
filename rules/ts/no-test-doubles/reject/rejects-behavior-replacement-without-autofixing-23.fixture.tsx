import { env } from "cloudflare:workers"; async function get() {} env.TOKEN = { get: get.bind(null).bind(null) };
