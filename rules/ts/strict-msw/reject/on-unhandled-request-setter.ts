import { setupServer } from "msw/node";
const network = setupServer(); network.listen({ set onUnhandledRequest(value) { record(value); } });
