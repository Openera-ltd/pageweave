import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Settings } from "../types";
import { SettingsDialog } from "./SettingsDialog";

const BASE: Settings = {
  version: 1,
  theme: "system",
  accent: "#2f6fde",
  density: "compact",
  ui_scale: 1,
  motion: "system",
  recent_files: [],
  shortcuts: {},
  restore_session: true,
  read_voice: null,
  read_rate: 1,
  check_updates: true,
};

afterEach(cleanup);

describe("Settings: markup colors", () => {
  it("shows the built-in colours, previews and saves a choice, and returns to the defaults", async () => {
    const onPreview = vi.fn();
    const onSave = vi.fn(() => Promise.resolve());
    render(<SettingsDialog initial={BASE} onPreview={onPreview} onSave={onSave} onCancel={() => {}} />);
    const highlight = screen.getByLabelText("Highlight color") as HTMLInputElement;
    expect(highlight.value).toBe("#ffd400");
    const reset = screen.getByRole("button", { name: "Default" });
    expect(reset.getAttribute("aria-disabled")).toBe("true");

    fireEvent.change(highlight, { target: { value: "#22cc88" } });
    expect(onPreview).toHaveBeenLastCalledWith(expect.objectContaining({ markup_colors: { highlight: "#22cc88" } }));
    expect(reset.getAttribute("aria-disabled")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenLastCalledWith(expect.objectContaining({ markup_colors: { highlight: "#22cc88" } }));

    fireEvent.click(reset);
    expect(onPreview).toHaveBeenLastCalledWith(expect.objectContaining({ markup_colors: {} }));
    expect((screen.getByLabelText("Highlight color") as HTMLInputElement).value).toBe("#ffd400");
  });

  it("starts from colours already chosen", () => {
    render(<SettingsDialog initial={{ ...BASE, markup_colors: { strike_out: "#000000" } }} onPreview={() => {}} onSave={() => Promise.resolve()} onCancel={() => {}} />);
    expect((screen.getByLabelText("Strikethrough color") as HTMLInputElement).value).toBe("#000000");
    expect((screen.getByLabelText("Underline color") as HTMLInputElement).value).toBe("#1f6feb");
  });
});
