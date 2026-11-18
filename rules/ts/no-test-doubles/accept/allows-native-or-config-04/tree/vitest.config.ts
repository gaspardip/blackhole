import { http, getResponse } from "msw"; export default { serviceBindings: { API: (request) => getResponse([http.all("*", () => new Response())], request) } };
