import {nativeBinding} from "./bindings"; export default {serviceBindings: {API: nativeBinding}, outboundService: request => fetch(request)};
