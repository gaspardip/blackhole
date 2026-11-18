import {setupServer} from "msw/node"; const options = {onUnhandledRequest: "error"}; setupServer().listen(options);
