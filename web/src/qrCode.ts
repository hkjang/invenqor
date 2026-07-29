import qrcode from "qrcode-generator";

// Enrolling a second factor meant reading an otpauth:// URI off the screen and
// typing it, or copying a base32 secret by hand, because the setup panel showed
// the text and no code. Every authenticator app expects a camera scan; the text
// was the fallback presented as the primary path.
//
// The matrix is rendered as an inline SVG path so the page stays self-contained -
// no canvas, no image request, and it prints and scales cleanly.

export type QRMatrix = {
  size: number;
  /** True where the module is dark. Row-major, size × size. */
  modules: boolean[];
};

/**
 * Encodes text as a QR matrix. Error-correction level M is the level
 * authenticator apps are documented against and tolerates a partly obscured or
 * poorly lit screen.
 */
export const qrMatrix = (text: string): QRMatrix => {
  // Type 0 asks the encoder for the smallest version that fits the payload.
  const code = qrcode(0, "M");
  code.addData(text);
  code.make();
  const size = code.getModuleCount();
  const modules: boolean[] = new Array(size * size);
  for (let row = 0; row < size; row++) {
    for (let column = 0; column < size; column++) {
      modules[row * size + column] = code.isDark(row, column);
    }
  }
  return {size, modules};
};

/**
 * One SVG path covering every dark module. A single path keeps the element
 * count independent of the payload length, which matters because a QR for a
 * long otpauth URI holds well over a thousand modules.
 */
export const qrPath = (matrix: QRMatrix): string => {
  const parts: string[] = [];
  for (let row = 0; row < matrix.size; row++) {
    let run = 0;
    for (let column = 0; column <= matrix.size; column++) {
      const dark = column < matrix.size &&
        matrix.modules[row * matrix.size + column];
      if (dark) {
        run += 1;
        continue;
      }
      if (run > 0) {
        parts.push(`M${column - run} ${row}h${run}v1h-${run}z`);
        run = 0;
      }
    }
  }
  return parts.join("");
};

/** The viewBox for a matrix plus the four-module quiet zone the format requires. */
export const qrViewBox = (matrix: QRMatrix, quietZone = 4): string =>
  `${-quietZone} ${-quietZone} ${matrix.size + quietZone * 2} ${
    matrix.size + quietZone * 2
  }`;

/**
 * Rasterises the matrix as RGBA pixels. Used by the test to decode its own
 * output with an independent decoder, because a QR code that renders but does
 * not scan fails silently in the one place it matters: someone else's phone.
 */
export const qrPixels = (
  matrix: QRMatrix,
  scale = 4,
  quietZone = 4,
): {data: Uint8ClampedArray; width: number; height: number} => {
  const modules = matrix.size + quietZone * 2;
  const width = modules * scale;
  const data = new Uint8ClampedArray(width * width * 4);
  for (let y = 0; y < width; y++) {
    for (let x = 0; x < width; x++) {
      const row = Math.floor(y / scale) - quietZone;
      const column = Math.floor(x / scale) - quietZone;
      const dark = row >= 0 && column >= 0 &&
        row < matrix.size && column < matrix.size &&
        matrix.modules[row * matrix.size + column];
      const offset = (y * width + x) * 4;
      const value = dark ? 0 : 255;
      data[offset] = value;
      data[offset + 1] = value;
      data[offset + 2] = value;
      data[offset + 3] = 255;
    }
  }
  return {data, width, height: width};
};
