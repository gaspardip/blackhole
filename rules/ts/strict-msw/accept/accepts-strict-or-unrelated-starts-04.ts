import { setupServer } from "msw/node";
const network = setupServer(); network.listen({ onUnhandledRequest: (request) => { unhandled.push(request.url); } });
