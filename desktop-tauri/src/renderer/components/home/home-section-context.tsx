import React, { createContext, useCallback, useContext, useMemo, useState } from "react";

import {
  readStoredHomeSection,
  storeHomeSection,
  type HomeSection,
} from "@/shared/home-sections";

const HomeSectionContext = createContext<{
  section: HomeSection;
  setSection: (section: HomeSection) => void;
} | null>(null);

export function HomeSectionProvider({ children }: { children: React.ReactNode }) {
  const [section, setSectionState] = useState<HomeSection>(() => readStoredHomeSection());

  const setSection = useCallback((next: HomeSection) => {
    setSectionState(next);
    storeHomeSection(next);
  }, []);

  const value = useMemo(() => ({ section, setSection }), [section, setSection]);

  return <HomeSectionContext.Provider value={value}>{children}</HomeSectionContext.Provider>;
}

export function useHomeSection(): {
  section: HomeSection;
  setSection: (section: HomeSection) => void;
} {
  const ctx = useContext(HomeSectionContext);
  if (!ctx) throw new Error("useHomeSection must be used inside HomeSectionProvider");
  return ctx;
}
