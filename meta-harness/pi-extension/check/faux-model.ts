// A scripted model for the manual check with real pi; it needs no network or credentials.
// The first prompt runs one command, the second tries another; the check allows the first and
// rejects the second.
import {
  fauxAssistantMessage,
  fauxProvider,
  fauxText,
  fauxThinking,
  fauxToolCall,
} from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function fauxModel(pi: ExtensionAPI): void {
  const faux = fauxProvider({ provider: "faux", models: [{ id: "faux-1" }] });
  faux.setResponses([
    fauxAssistantMessage([
      fauxThinking("Plan it."),
      fauxText("Running it."),
      fauxToolCall("bash", { command: "echo approved-run" }),
    ]),
    fauxAssistantMessage("Done."),
    fauxAssistantMessage([fauxToolCall("bash", { command: "echo rejected-run" })]),
    fauxAssistantMessage("Understood."),
  ]);
  pi.registerProvider(faux.provider);
}
