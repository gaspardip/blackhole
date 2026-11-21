import { http, getResponse } from "msw"; const handlers = [http.get("*", () => Response.json({}))]; export default { outboundService: (request) => getResponse(handlers, request) };
