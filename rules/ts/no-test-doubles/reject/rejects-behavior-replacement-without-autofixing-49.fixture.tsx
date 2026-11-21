import { env as bindings } from "cloudflare:workers"; bindings.AUTH.verify = () => null;
