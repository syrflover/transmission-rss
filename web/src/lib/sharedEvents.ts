import { createEvents, type EventSourceLike, type Events, type Visibility } from "./events";

/** The event stream of the server (`src/web/events_api.rs`). */
const EVENTS_URL = "/api/events";

function browserSource(): EventSourceLike {
  const source = new EventSource(EVENTS_URL);
  return {
    addEventListener: (type, listener) => source.addEventListener(type, (event) => listener(event as MessageEvent)),
    close: () => source.close(),
    get readyState() {
      return source.readyState;
    },
  };
}

const documentVisibility: Visibility = {
  hidden: () => document.hidden,
  onChange: (listener) => {
    document.addEventListener("visibilitychange", listener);
    return () => document.removeEventListener("visibilitychange", listener);
  },
};

let shared: Events | null = null;

/** The tab's one connection to the event stream, made when the first screen asks for it. */
export function sharedEvents(): Events {
  shared ??= createEvents({ open: browserSource, visibility: documentVisibility });
  return shared;
}
