import { useApi } from "../api";
import { CSS_PX_PER_PT, displaySizePt, pdfRectToScreen } from "../geometry";
import type { PageInfo } from "../types";

interface Props {
  docId: number;
  page: PageInfo;
  /** [x1, y1, x2, y2] in the page's unrotated PDF space. */
  rect: [number, number, number, number];
  /** Longest side of the thumbnail, CSS px. */
  size?: number;
}

/** A page thumbnail with the box where something will be placed. */
export function PlacementPreview({ docId, page, rect, size = 150 }: Props) {
  const api = useApi();
  const d = displaySizePt(page);
  const zoom = size / Math.max(d.w, d.h) / CSS_PX_PER_PT;
  const box = pdfRectToScreen({ left: rect[0], bottom: rect[1], right: rect[2], top: rect[3] }, page, zoom);
  const w = d.w * zoom * CSS_PX_PER_PT;
  const h = d.h * zoom * CSS_PX_PER_PT;
  const scale = Math.max(50, Math.round((size / Math.max(page.width_pt, page.height_pt)) * 1000 * (window.devicePixelRatio || 1)));
  return (
    <div className="sign-preview" style={{ width: w, height: h }} aria-hidden="true">
      <img src={api.pageUrl(docId, page.index, scale)} alt="" draggable={false} style={{ width: w, height: h }} />
      <div className="sign-preview-box" data-testid="placement-box" style={{ left: box.left, top: box.top, width: box.width, height: box.height }} />
    </div>
  );
}
