import { setupServer } from "msw/node";
const network = setupServer(); network.listen({ onUnhandledRequest: function (request, print) { print.error(); } });
