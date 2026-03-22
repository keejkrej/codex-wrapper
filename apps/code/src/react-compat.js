import ReactCompat from "preact/compat";

export * from "preact/compat";
export default ReactCompat;

export function use(value) {
  if (value && typeof value.then === "function") {
    throw value;
  }
  return value;
}

export function useOptimistic(initialState, updateFn) {
  const [state, setState] = ReactCompat.useState(initialState);

  ReactCompat.useEffect(() => {
    setState(initialState);
  }, [initialState, setState]);

  const setOptimisticState = (value) => {
    setState((currentState) => {
      const resolvedValue = typeof value === "function" ? value(currentState) : value;
      return updateFn ? updateFn(currentState, resolvedValue) : resolvedValue;
    });
  };

  return [state, setOptimisticState];
}
