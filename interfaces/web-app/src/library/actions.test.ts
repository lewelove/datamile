import { describe, expect, test } from "bun:test";
import { actionPresentation, hasAction } from "./actions.ts";
import type { ActionMap } from "../types.ts";

const actions: ActionMap = {
  open_config_in_terminal: {
    label: "Open Config in Terminal",
    description: "Open terminal inside the config directory"
  },
  open_terminal: {
    label: "open_terminal",
    media: {
      album: { label: "Open Album Terminal", description: "Open a terminal in the album dir" },
      movie: { label: "Open Movie Terminal" }
    }
  }
};

describe("hasAction", () => {
  test("generic actions apply to every media type, including none", () => {
    expect(hasAction(actions, "open_config_in_terminal", null)).toBe(true);
    expect(hasAction(actions, "open_config_in_terminal", "album")).toBe(true);
    expect(hasAction(actions, "open_config_in_terminal", "movie")).toBe(true);
  });

  test("scoped actions apply only to their declared media type", () => {
    expect(hasAction(actions, "open_terminal", "album")).toBe(true);
    expect(hasAction(actions, "open_terminal", "movie")).toBe(true);
    expect(hasAction(actions, "open_terminal", "book")).toBe(false);
    expect(hasAction(actions, "open_terminal", null)).toBe(false);
  });

  test("unknown actions are absent", () => {
    expect(hasAction(actions, "missing", "album")).toBe(false);
  });
});

describe("actionPresentation", () => {
  test("generic actions return the flat label and description", () => {
    expect(actionPresentation(actions, "open_config_in_terminal", "album")).toEqual({
      label: "Open Config in Terminal",
      description: "Open terminal inside the config directory"
    });
  });

  test("scoped actions return the presentation for the matching media type", () => {
    expect(actionPresentation(actions, "open_terminal", "album")).toEqual({
      label: "Open Album Terminal",
      description: "Open a terminal in the album dir"
    });
    expect(actionPresentation(actions, "open_terminal", "movie")).toEqual({
      label: "Open Movie Terminal"
    });
  });

  test("scoped actions without a matching media type return nothing", () => {
    expect(actionPresentation(actions, "open_terminal", "book")).toBeNull();
    expect(actionPresentation(actions, "open_terminal", null)).toBeNull();
  });

  test("unknown actions return nothing", () => {
    expect(actionPresentation(actions, "missing", "album")).toBeNull();
  });
});
