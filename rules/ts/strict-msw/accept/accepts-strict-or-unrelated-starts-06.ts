import { setupServer } from "msw/node";
const network = setupServer(); network.close(); network.resetHandlers();
