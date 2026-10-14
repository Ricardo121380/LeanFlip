import { createContext, useContext, useEffect, useRef } from "react";

export interface EditorLeaveState { dirty: boolean; saving: boolean; discard: () => void }
export const EditorNavigationContext = createContext<Set<{ current: EditorLeaveState }> | null>(null);

export function useEditorNavigation(state: EditorLeaveState) {
  const guards = useContext(EditorNavigationContext);
  const ref = useRef(state);
  ref.current = state;
  useEffect(() => {
    guards?.add(ref);
    return () => { guards?.delete(ref); };
  }, [guards]);
}
