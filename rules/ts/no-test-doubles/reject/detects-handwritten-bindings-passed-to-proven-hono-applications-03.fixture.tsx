import { OpenAPIHono } from "@hono/zod-openapi"; function app() { return new OpenAPIHono().route("/", routes); } app().request("/", {}, {AUTH_SERVICE: {authorize: () => true}});
