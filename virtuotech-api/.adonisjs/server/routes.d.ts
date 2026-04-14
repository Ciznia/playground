import '@adonisjs/core/types/http'

type ParamValue = string | number | bigint | boolean

export type ScannedRoutes = {
  ALL: {
    'openapi.html': { paramsTuple?: []; params?: {} }
    'openapi.json': { paramsTuple?: []; params?: {} }
    'openapi.yaml': { paramsTuple?: []; params?: {} }
    'google_oauth.google_redirect': { paramsTuple?: []; params?: {} }
    'google_oauth.google_callback': { paramsTuple?: []; params?: {} }
  }
  GET: {
    'openapi.html': { paramsTuple?: []; params?: {} }
    'openapi.json': { paramsTuple?: []; params?: {} }
    'openapi.yaml': { paramsTuple?: []; params?: {} }
    'google_oauth.google_redirect': { paramsTuple?: []; params?: {} }
    'google_oauth.google_callback': { paramsTuple?: []; params?: {} }
  }
  HEAD: {
    'openapi.html': { paramsTuple?: []; params?: {} }
    'openapi.json': { paramsTuple?: []; params?: {} }
    'openapi.yaml': { paramsTuple?: []; params?: {} }
    'google_oauth.google_redirect': { paramsTuple?: []; params?: {} }
    'google_oauth.google_callback': { paramsTuple?: []; params?: {} }
  }
}
declare module '@adonisjs/core/types/http' {
  export interface RoutesList extends ScannedRoutes {}
}