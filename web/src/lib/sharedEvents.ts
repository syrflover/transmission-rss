import { createEvents, type EventSourceLike, type Events, type Returns, type Visibility } from "./events";

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

/** The window's signs that the page was away: `pageshow`, `focus`, `online`. */
const windowReturns: Returns = {
  onReturn: (listener) => {
    const signs = ["pageshow", "focus", "online"] as const;
    for (const sign of signs) window.addEventListener(sign, listener);
    return () => {
      for (const sign of signs) window.removeEventListener(sign, listener);
    };
  },
};

let shared: Events | null = null;

/** The tab's one connection to the event stream, made when the first screen asks for it. */
export function sharedEvents(): Events {
  shared ??= createEvents({ open: browserSource, visibility: documentVisibility, returns: windowReturns });
  return shared;
}
