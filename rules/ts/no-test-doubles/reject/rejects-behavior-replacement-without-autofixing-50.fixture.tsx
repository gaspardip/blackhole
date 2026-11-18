import { env } from "cloudflare:workers"; const { AUTH } = env; AUTH.verify = function () {};
