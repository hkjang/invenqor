import {describe, expect, it} from "vitest";
import jsQR from "jsqr";
import {qrMatrix, qrPath, qrPixels, qrViewBox} from "./qrCode";

// A QR code that renders but does not scan fails on someone else's phone, where
// nobody can debug it. So the test decodes the rendered output with an
// independent decoder rather than checking that a matrix was produced.
describe("qrMatrix", () => {
  it("round-trips a provisioning URI through a real decoder", () => {
    const uri = "otpauth://totp/Invenqor%3Aconsole.admin" +
      "?algorithm=SHA1&digits=6&issuer=Invenqor&period=30" +
      "&secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
    const matrix = qrMatrix(uri);
    const {data, width, height} = qrPixels(matrix, 6);
    const decoded = jsQR(data, width, height);
    expect(decoded?.data).toBe(uri);
  });

  it("stays scannable for the longest realistic label", () => {
    const uri = "otpauth://totp/" +
      encodeURIComponent("Invenqor:operations.administrator@subsidiary.example.com") +
      "?algorithm=SHA1&digits=6&issuer=Invenqor&period=30" +
      "&secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXPJBSWY3DP";
    const matrix = qrMatrix(uri);
    const {data, width, height} = qrPixels(matrix, 6);
    expect(jsQR(data, width, height)?.data).toBe(uri);
  });
});

describe("qrPath", () => {
  it("covers exactly the dark modules", () => {
    const matrix = qrMatrix("invenqor");
    const path = qrPath(matrix);
    // Each command is one horizontal run, so the widths must add up to the
    // number of dark modules - a run merged or dropped would change the total.
    const covered = [...path.matchAll(/h(\d+)/g)]
      .reduce((sum, match) => sum + Number(match[1]), 0);
    const dark = matrix.modules.filter(Boolean).length;
    expect(covered).toBe(dark);
  });

  it("merges neighbouring modules into one run", () => {
    // A path with one command per module would emit thousands of commands.
    const matrix = qrMatrix("invenqor");
    const commands = qrPath(matrix).match(/M/g)?.length ?? 0;
    expect(commands).toBeLessThan(matrix.modules.filter(Boolean).length);
  });

  it("reserves the quiet zone a scanner needs", () => {
    const matrix = qrMatrix("invenqor");
    expect(qrViewBox(matrix)).toBe(`-4 -4 ${matrix.size + 8} ${matrix.size + 8}`);
  });
});
