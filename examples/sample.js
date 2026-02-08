/**
 * Reactive state management store with middleware support.
 *
 * Implements a Redux-like pattern with immer-style immutable updates,
 * computed selectors, and an effect/middleware pipeline.
 */

// --- Core Store ---

const LISTENERS = Symbol("listeners");
const MIDDLEWARE = Symbol("middleware");

export function createStore(initialState, options = {}) {
  let state = structuredClone(initialState);
  const listeners = new Set();
  const middlewares = options.middleware ?? [];
  const devtools = options.devtools ?? false;
  const history = devtools ? [{ state: structuredClone(state), action: "@@INIT", ts: Date.now() }] : null;

  function getState() {
    return state;
  }

  function dispatch(action) {
    const prev = state;

    // Run middleware chain
    const chain = middlewares.reduceRight(
      (next, mw) => (act) => mw({ getState, dispatch })(next)(act),
      (act) => {
        if (typeof act === "function") {
          // Thunk support
          return act(dispatch, getState);
        }
        state = rootReducer(state, act);
        return act;
      }
    );

    const result = chain(action);

    if (state !== prev) {
      if (history) {
        history.push({
          state: structuredClone(state),
          action: typeof action === "string" ? action : action.type,
          ts: Date.now(),
        });
      }
      listeners.forEach((fn) => {
        try {
          fn(state, prev);
        } catch (err) {
          console.error("[store] Listener error:", err);
        }
      });
    }

    return result;
  }

  function subscribe(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
  }

  function select(selector) {
    return selector(state);
  }

  return { getState, dispatch, subscribe, select, [LISTENERS]: listeners, [MIDDLEWARE]: middlewares };
}

// --- Reducer Combinators ---

const reducerRegistry = new Map();

export function registerReducer(key, reducer) {
  reducerRegistry.set(key, reducer);
}

function rootReducer(state, action) {
  let nextState = state;
  let changed = false;

  for (const [key, reducer] of reducerRegistry) {
    const prev = state[key];
    const next = reducer(prev, action);
    if (next !== prev) {
      if (!changed) {
        nextState = { ...state };
        changed = true;
      }
      nextState[key] = next;
    }
  }

  return nextState;
}

// --- Built-in Middleware ---

export function loggerMiddleware({ getState }) {
  return (next) => (action) => {
    const type = typeof action === "string" ? action : action?.type ?? "thunk";
    console.group(`%c[dispatch] ${type}`, "color: #6366f1; font-weight: bold");
    console.log("%cprev state", "color: #9ca3af", getState());
    const result = next(action);
    console.log("%cnext state", "color: #22c55e", getState());
    console.groupEnd();
    return result;
  };
}

export function thunkMiddleware({ dispatch, getState }) {
  return (next) => (action) => {
    if (typeof action === "function") {
      return action(dispatch, getState);
    }
    return next(action);
  };
}

export function batchMiddleware() {
  let queue = [];
  let flushing = false;

  return ({ dispatch }) =>
    (next) =>
    (action) => {
      if (action?.type === "@@BATCH_START") {
        queue = [];
        return;
      }

      if (action?.type === "@@BATCH_END") {
        flushing = true;
        try {
          queue.forEach((a) => next(a));
        } finally {
          queue = [];
          flushing = false;
        }
        return;
      }

      if (queue.length > 0 && !flushing) {
        queue.push(action);
        return;
      }

      return next(action);
    };
}

// --- Selector Utilities ---

export function createSelector(deps, compute) {
  let lastArgs = null;
  let lastResult = null;

  return (state) => {
    const args = deps.map((dep) => dep(state));
    const changed = lastArgs === null || args.some((arg, i) => arg !== lastArgs[i]);

    if (changed) {
      lastResult = compute(...args);
      lastArgs = args;
    }

    return lastResult;
  };
}

// --- Example: E-commerce Domain ---

const CartActionTypes = {
  ADD_ITEM: "cart/addItem",
  REMOVE_ITEM: "cart/removeItem",
  UPDATE_QUANTITY: "cart/updateQuantity",
  APPLY_COUPON: "cart/applyCoupon",
  CLEAR: "cart/clear",
};

function cartReducer(state = { items: [], coupon: null }, action) {
  switch (action.type) {
    case CartActionTypes.ADD_ITEM: {
      const existing = state.items.find((i) => i.sku === action.payload.sku);
      if (existing) {
        return {
          ...state,
          items: state.items.map((i) =>
            i.sku === action.payload.sku ? { ...i, qty: i.qty + (action.payload.qty ?? 1) } : i
          ),
        };
      }
      return { ...state, items: [...state.items, { ...action.payload, qty: action.payload.qty ?? 1 }] };
    }

    case CartActionTypes.REMOVE_ITEM:
      return { ...state, items: state.items.filter((i) => i.sku !== action.payload.sku) };

    case CartActionTypes.UPDATE_QUANTITY:
      return {
        ...state,
        items: state.items
          .map((i) => (i.sku === action.payload.sku ? { ...i, qty: action.payload.qty } : i))
          .filter((i) => i.qty > 0),
      };

    case CartActionTypes.APPLY_COUPON:
      return { ...state, coupon: action.payload };

    case CartActionTypes.CLEAR:
      return { items: [], coupon: null };

    default:
      return state;
  }
}

registerReducer("cart", cartReducer);

// --- Selectors ---

const selectCart = (state) => state.cart;
const selectCartItems = (state) => state.cart?.items ?? [];

export const selectCartTotal = createSelector([selectCartItems], (items) =>
  items.reduce((sum, item) => sum + item.priceCents * item.qty, 0)
);

export const selectCartItemCount = createSelector([selectCartItems], (items) =>
  items.reduce((sum, item) => sum + item.qty, 0)
);

export const selectCartWithDiscount = createSelector(
  [selectCartTotal, selectCart],
  (total, cart) => {
    if (!cart.coupon) return { total, discount: 0, final: total };
    const discount = cart.coupon.type === "percent"
      ? Math.round(total * (cart.coupon.value / 100))
      : cart.coupon.value;
    return { total, discount, final: Math.max(0, total - discount) };
  }
);

// --- Async Action Creators (Thunks) ---

export function addToCart(sku, qty = 1) {
  return async (dispatch, getState) => {
    dispatch({ type: "cart/loading", payload: { sku } });

    try {
      const res = await fetch(`/api/v2/inventory/${sku}/check`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ qty }),
      });

      if (!res.ok) {
        const error = await res.json();
        dispatch({ type: "cart/error", payload: { sku, message: error.message } });
        return;
      }

      const { product, available } = await res.json();

      if (available < qty) {
        dispatch({
          type: "cart/error",
          payload: { sku, message: `Only ${available} units available` },
        });
        return;
      }

      dispatch({
        type: CartActionTypes.ADD_ITEM,
        payload: { sku, name: product.name, priceCents: product.priceCents, qty },
      });
    } catch (err) {
      dispatch({ type: "cart/error", payload: { sku, message: err.message } });
    }
  };
}

export function checkout() {
  return async (dispatch, getState) => {
    const { cart } = getState();
    if (cart.items.length === 0) return;

    dispatch({ type: "checkout/started" });

    try {
      const res = await fetch("/api/v2/orders", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          items: cart.items.map(({ sku, qty }) => ({ sku, qty })),
          coupon: cart.coupon?.code ?? null,
        }),
      });

      if (!res.ok) throw new Error(`Order failed: ${res.status}`);

      const order = await res.json();
      dispatch({ type: "checkout/success", payload: order });
      dispatch({ type: CartActionTypes.CLEAR });
    } catch (err) {
      dispatch({ type: "checkout/failed", payload: { message: err.message } });
    }
  };
}
