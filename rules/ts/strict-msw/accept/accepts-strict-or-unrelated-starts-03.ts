import { setupServer } from "msw/node";
const network = setupServer(); network.listen({ onUnhandledRequest(request, print) { print.error(); } });
