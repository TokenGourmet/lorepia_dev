import {
  contentSwipeBack,
  type ContentBackOptions,
} from "./content-back";
import { activateLatestBackSwipeSurface } from "./back-swipe-surface";

const EDGE_START = 24;

export interface EdgeBackOptions {
  onBack: () => void;
  getUnderlay?: () => HTMLElement | null;
  enabled?: boolean;
}

/**
 * Edge-only compatibility action for detail screens. It deliberately delegates
 * to the same transition engine as chat's content-wide gesture so Android
 * native progress, desktop mouse fallback, corner curvature, underlay parallax,
 * and reduced-motion behavior cannot diverge.
 */
export function edgeSwipeBack(
  node: HTMLElement,
  initialOptions: EdgeBackOptions,
): { update(options: EdgeBackOptions): void; destroy(): void } {
  const toContentOptions = (
    options: EdgeBackOptions,
  ): ContentBackOptions => ({
    ...options,
    getUnderlay:
      options.getUnderlay ?? activateLatestBackSwipeSurface,
    edgeWidth: EDGE_START,
  });
  const action = contentSwipeBack(node, toContentOptions(initialOptions));

  return {
    update(options): void {
      action.update(toContentOptions(options));
    },
    destroy(): void {
      action.destroy();
    },
  };
}
