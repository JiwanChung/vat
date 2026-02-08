import React, { useState, useEffect, useCallback, useMemo, useRef, Suspense, lazy } from "react";
import { createPortal } from "react-dom";
import PropTypes from "prop-types";

// Lazy-loaded components
const AnalyticsChart = lazy(() => import("./AnalyticsChart"));
const ExportDialog = lazy(() => import("./ExportDialog"));

// --- Custom Hooks ---

function useDebounce(value, delay = 300) {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}

function useIntersectionObserver(ref, options = {}) {
  const [entry, setEntry] = useState(null);
  useEffect(() => {
    if (!ref.current) return;
    const observer = new IntersectionObserver(([e]) => setEntry(e), {
      threshold: 0.1,
      rootMargin: "50px",
      ...options,
    });
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, [ref, options]);
  return entry;
}

function useLocalStorage(key, initialValue) {
  const [stored, setStored] = useState(() => {
    try {
      const item = window.localStorage.getItem(key);
      return item ? JSON.parse(item) : initialValue;
    } catch {
      return initialValue;
    }
  });

  const setValue = useCallback(
    (value) => {
      const next = value instanceof Function ? value(stored) : value;
      setStored(next);
      window.localStorage.setItem(key, JSON.stringify(next));
    },
    [key, stored]
  );

  return [stored, setValue];
}

// --- Context ---

const ThemeContext = React.createContext({ mode: "light", toggle: () => {} });
const NotificationContext = React.createContext({ notifications: [], push: () => {}, dismiss: () => {} });

// --- Utility Components ---

function ErrorBoundary({ children, fallback }) {
  const [error, setError] = useState(null);

  if (error) {
    return fallback ? fallback({ error, reset: () => setError(null) }) : (
      <div role="alert" className="error-boundary">
        <h2>Something went wrong</h2>
        <pre>{error.message}</pre>
        <button onClick={() => setError(null)}>Try Again</button>
      </div>
    );
  }

  return (
    <React.Fragment>
      {React.Children.map(children, (child) =>
        React.cloneElement(child, {
          onError: (err) => setError(err),
        })
      )}
    </React.Fragment>
  );
}

ErrorBoundary.propTypes = {
  children: PropTypes.node.isRequired,
  fallback: PropTypes.func,
};

function Tooltip({ children, content, position = "top" }) {
  const [visible, setVisible] = useState(false);
  const ref = useRef(null);
  const [coords, setCoords] = useState({ x: 0, y: 0 });

  const show = useCallback(() => {
    if (ref.current) {
      const rect = ref.current.getBoundingClientRect();
      const positions = {
        top: { x: rect.left + rect.width / 2, y: rect.top - 8 },
        bottom: { x: rect.left + rect.width / 2, y: rect.bottom + 8 },
        left: { x: rect.left - 8, y: rect.top + rect.height / 2 },
        right: { x: rect.right + 8, y: rect.top + rect.height / 2 },
      };
      setCoords(positions[position] || positions.top);
    }
    setVisible(true);
  }, [position]);

  return (
    <>
      <span ref={ref} onMouseEnter={show} onMouseLeave={() => setVisible(false)}>
        {children}
      </span>
      {visible &&
        createPortal(
          <div
            className={`tooltip tooltip--${position}`}
            style={{ left: coords.x, top: coords.y }}
            role="tooltip"
          >
            {content}
          </div>,
          document.body
        )}
    </>
  );
}

// --- Main Data Table Component ---

const SORT_DIRECTIONS = { ASC: "asc", DESC: "desc" };
const PAGE_SIZES = [10, 25, 50, 100];

function DataTable({ columns, data, onRowSelect, selectable = false, stickyHeader = true }) {
  const [sortCol, setSortCol] = useState(null);
  const [sortDir, setSortDir] = useState(SORT_DIRECTIONS.ASC);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [pageSize, setPageSize] = useLocalStorage("table-page-size", 25);
  const [selected, setSelected] = useState(new Set());
  const [columnFilters, setColumnFilters] = useState({});
  const tableRef = useRef(null);

  const debouncedSearch = useDebounce(search);

  // Filter data
  const filtered = useMemo(() => {
    let result = data;
    if (debouncedSearch) {
      const lower = debouncedSearch.toLowerCase();
      result = result.filter((row) =>
        columns.some((col) => String(row[col.key]).toLowerCase().includes(lower))
      );
    }
    Object.entries(columnFilters).forEach(([key, filterValue]) => {
      if (filterValue) {
        result = result.filter((row) =>
          String(row[key]).toLowerCase().includes(filterValue.toLowerCase())
        );
      }
    });
    return result;
  }, [data, debouncedSearch, columns, columnFilters]);

  // Sort data
  const sorted = useMemo(() => {
    if (!sortCol) return filtered;
    return [...filtered].sort((a, b) => {
      const aVal = a[sortCol];
      const bVal = b[sortCol];
      const cmp = typeof aVal === "number" ? aVal - bVal : String(aVal).localeCompare(String(bVal));
      return sortDir === SORT_DIRECTIONS.ASC ? cmp : -cmp;
    });
  }, [filtered, sortCol, sortDir]);

  // Paginate
  const totalPages = Math.ceil(sorted.length / pageSize);
  const pageData = sorted.slice(page * pageSize, (page + 1) * pageSize);

  const handleSort = useCallback(
    (colKey) => {
      if (sortCol === colKey) {
        setSortDir((d) => (d === SORT_DIRECTIONS.ASC ? SORT_DIRECTIONS.DESC : SORT_DIRECTIONS.ASC));
      } else {
        setSortCol(colKey);
        setSortDir(SORT_DIRECTIONS.ASC);
      }
    },
    [sortCol]
  );

  const toggleRow = useCallback(
    (id) => {
      setSelected((prev) => {
        const next = new Set(prev);
        next.has(id) ? next.delete(id) : next.add(id);
        onRowSelect?.(Array.from(next));
        return next;
      });
    },
    [onRowSelect]
  );

  const toggleAll = useCallback(() => {
    if (selected.size === pageData.length) {
      setSelected(new Set());
      onRowSelect?.([]);
    } else {
      const all = new Set(pageData.map((r) => r.id));
      setSelected(all);
      onRowSelect?.(Array.from(all));
    }
  }, [pageData, selected, onRowSelect]);

  // Keyboard navigation
  useEffect(() => {
    const handler = (e) => {
      if (e.key === "ArrowLeft" && page > 0) setPage((p) => p - 1);
      if (e.key === "ArrowRight" && page < totalPages - 1) setPage((p) => p + 1);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [page, totalPages]);

  return (
    <div className="data-table-wrapper" ref={tableRef}>
      <div className="data-table-toolbar">
        <input
          type="search"
          placeholder="Search all columns..."
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setPage(0);
          }}
          aria-label="Search table"
        />
        <span className="result-count">
          {filtered.length} of {data.length} rows
          {selected.size > 0 && ` (${selected.size} selected)`}
        </span>
        <select
          value={pageSize}
          onChange={(e) => {
            setPageSize(Number(e.target.value));
            setPage(0);
          }}
        >
          {PAGE_SIZES.map((s) => (
            <option key={s} value={s}>
              {s} per page
            </option>
          ))}
        </select>
      </div>

      <table className={stickyHeader ? "sticky-header" : ""} role="grid">
        <thead>
          <tr>
            {selectable && (
              <th>
                <input
                  type="checkbox"
                  checked={selected.size === pageData.length && pageData.length > 0}
                  onChange={toggleAll}
                  aria-label="Select all rows"
                />
              </th>
            )}
            {columns.map((col) => (
              <th
                key={col.key}
                onClick={() => col.sortable !== false && handleSort(col.key)}
                className={col.sortable !== false ? "sortable" : ""}
                aria-sort={
                  sortCol === col.key
                    ? sortDir === SORT_DIRECTIONS.ASC
                      ? "ascending"
                      : "descending"
                    : "none"
                }
              >
                <div className="th-content">
                  <span>{col.label}</span>
                  {sortCol === col.key && (
                    <span className="sort-icon">
                      {sortDir === SORT_DIRECTIONS.ASC ? "\u25B2" : "\u25BC"}
                    </span>
                  )}
                </div>
                {col.filterable && (
                  <input
                    type="text"
                    className="column-filter"
                    placeholder={`Filter ${col.label}...`}
                    value={columnFilters[col.key] || ""}
                    onChange={(e) =>
                      setColumnFilters((prev) => ({ ...prev, [col.key]: e.target.value }))
                    }
                    onClick={(e) => e.stopPropagation()}
                  />
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {pageData.length === 0 ? (
            <tr>
              <td colSpan={columns.length + (selectable ? 1 : 0)} className="empty-state">
                No matching records found
              </td>
            </tr>
          ) : (
            pageData.map((row, idx) => (
              <tr
                key={row.id || idx}
                className={selected.has(row.id) ? "selected" : ""}
                onClick={() => selectable && toggleRow(row.id)}
              >
                {selectable && (
                  <td>
                    <input
                      type="checkbox"
                      checked={selected.has(row.id)}
                      onChange={() => toggleRow(row.id)}
                    />
                  </td>
                )}
                {columns.map((col) => (
                  <td key={col.key} className={col.align ? `align-${col.align}` : ""}>
                    {col.render ? col.render(row[col.key], row) : row[col.key]}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>

      <div className="pagination" role="navigation" aria-label="Table pagination">
        <button disabled={page === 0} onClick={() => setPage(0)}>
          &laquo; First
        </button>
        <button disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
          &lsaquo; Prev
        </button>
        <span>
          Page {page + 1} of {totalPages || 1}
        </span>
        <button disabled={page >= totalPages - 1} onClick={() => setPage((p) => p + 1)}>
          Next &rsaquo;
        </button>
        <button disabled={page >= totalPages - 1} onClick={() => setPage(totalPages - 1)}>
          Last &raquo;
        </button>
      </div>
    </div>
  );
}

DataTable.propTypes = {
  columns: PropTypes.arrayOf(
    PropTypes.shape({
      key: PropTypes.string.isRequired,
      label: PropTypes.string.isRequired,
      sortable: PropTypes.bool,
      filterable: PropTypes.bool,
      align: PropTypes.oneOf(["left", "center", "right"]),
      render: PropTypes.func,
    })
  ).isRequired,
  data: PropTypes.arrayOf(PropTypes.object).isRequired,
  onRowSelect: PropTypes.func,
  selectable: PropTypes.bool,
  stickyHeader: PropTypes.bool,
};

// --- App Component ---

export default function App() {
  const [theme, setTheme] = useLocalStorage("theme", "light");
  const [notifications, setNotifications] = useState([]);

  const pushNotification = useCallback((msg, type = "info") => {
    const id = Date.now();
    setNotifications((prev) => [...prev, { id, msg, type }]);
    setTimeout(() => {
      setNotifications((prev) => prev.filter((n) => n.id !== id));
    }, 5000);
  }, []);

  const themeValue = useMemo(
    () => ({
      mode: theme,
      toggle: () => setTheme((t) => (t === "light" ? "dark" : "light")),
    }),
    [theme, setTheme]
  );

  return (
    <ThemeContext.Provider value={themeValue}>
      <NotificationContext.Provider
        value={{
          notifications,
          push: pushNotification,
          dismiss: (id) => setNotifications((prev) => prev.filter((n) => n.id !== id)),
        }}
      >
        <div className={`app app--${theme}`} data-theme={theme}>
          <ErrorBoundary>
            <header className="app-header">
              <h1>Analytics Dashboard</h1>
              <Tooltip content="Toggle dark/light mode">
                <button onClick={themeValue.toggle} aria-label="Toggle theme">
                  {theme === "light" ? "\uD83C\uDF19" : "\u2600\uFE0F"}
                </button>
              </Tooltip>
            </header>

            <main>
              <Suspense fallback={<div className="skeleton chart-skeleton" />}>
                <AnalyticsChart />
              </Suspense>

              <DataTable
                columns={[
                  { key: "name", label: "Name", filterable: true },
                  { key: "email", label: "Email", filterable: true },
                  { key: "role", label: "Role", filterable: true },
                  {
                    key: "revenue",
                    label: "Revenue",
                    align: "right",
                    render: (val) => `$${val.toLocaleString()}`,
                  },
                  {
                    key: "status",
                    label: "Status",
                    render: (val) => (
                      <span className={`badge badge--${val}`}>{val}</span>
                    ),
                  },
                ]}
                data={[]}
                selectable
                onRowSelect={(ids) => console.log("Selected:", ids)}
              />

              <Suspense fallback={<div className="skeleton" />}>
                <ExportDialog />
              </Suspense>
            </main>

            <div className="notification-container" aria-live="polite">
              {notifications.map((n) => (
                <div key={n.id} className={`notification notification--${n.type}`} role="alert">
                  {n.msg}
                </div>
              ))}
            </div>
          </ErrorBoundary>
        </div>
      </NotificationContext.Provider>
    </ThemeContext.Provider>
  );
}
