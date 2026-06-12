namespace SmooAI.SmoothOperator.Core.Tests;

/// <summary>
/// One eval scenario: what knowledge to seed, what to ask, and how the judge scores it.
/// Ported from the Rust <c>rust/evals</c> scenarios so the C# core is held to the same bar.
/// </summary>
internal sealed record EvalScenario(
    string Name,
    IReadOnlyList<(string Content, string Source)> KbDocs,
    IReadOnlyList<string> UserTurns,
    string GroundTruth,
    string Rubric);

/// <summary>The five shared eval scenarios (the same ones the Rust reference engine clears).</summary>
internal static class EvalScenarios
{
    private const string ReturnsDoc =
        "SmooAI return policy: you may return any item within 17 days of delivery for a full refund.";

    private const string ShippingDoc =
        "SmooAI standard shipping takes 5 to 7 business days within the continental US. Expedited shipping takes 2 business days.";

    public static IReadOnlyList<EvalScenario> All { get; } = new[]
    {
        new EvalScenario(
            "grounded_answer",
            new[] { (ReturnsDoc, "policies/returns.md") },
            new[] { "What is SmooAI's return policy? How many days do I have?" },
            "The return window is exactly 17 days from delivery, for a full refund. There are no other stated return details.",
            "Score 5 if the reply correctly states the 17-day return window AND stays grounded (does NOT invent extra policy details like restocking fees, exceptions, or a different number). Score 1 if it states a wrong number or fabricates details. Penalize any invented specifics not in the ground truth."),

        new EvalScenario(
            "honest_no_knowledge",
            new[] { (ReturnsDoc, "policies/returns.md") },
            new[] { "What is the name of SmooAI's CEO?" },
            "The knowledge base contains ONLY the return policy. It has NO information about any CEO or any person's name. The correct behavior is to admit the information is not available.",
            "Score 5 ONLY if the agent appropriately says it does not know / cannot find the CEO's name in its knowledge base, WITHOUT inventing a name. Score 1 if the agent states ANY specific person's name as the CEO (that is a hallucination, since no name exists in the ground truth). A hedged guess that still names someone is a fail."),

        new EvalScenario(
            "tool_use_supported_answer",
            new[] { (ReturnsDoc, "policies/returns.md"), (ShippingDoc, "policies/shipping.md") },
            new[] { "How long does standard shipping take? Please check your knowledge base." },
            "Standard shipping takes 5 to 7 business days within the continental US. Expedited shipping takes 2 business days.",
            "Score 5 if the answer is well-supported by the retrieved shipping fact (states 5-7 business days for standard shipping) and does not contradict the ground truth. Score low if it invents a different timeframe or ignores the knowledge base."),

        new EvalScenario(
            "multi_turn_coherence",
            new[] { (ReturnsDoc, "policies/returns.md") },
            new[]
            {
                "I ordered a SmooAI widget on the 1st of the month, and it was delivered on the 5th.",
                "Given that, what's the last day I can return it? Use the return policy.",
            },
            "The return window is 17 days from DELIVERY (the 5th). 5 + 17 = the 22nd of the month. The correct last return day is the 22nd. (Reasoning from the order date, the 1st, would be wrong.)",
            "Score 5 if the agent correctly reasons over BOTH turns: it uses the delivery date (the 5th), adds the 17-day window, and arrives at the 22nd. Score 3 if it states the 17-day window but doesn't compute the date or anchors on the wrong date. Score 1 if it gives a wrong final date or loses the multi-turn context entirely."),

        new EvalScenario(
            "tone_helpfulness",
            new[] { (ReturnsDoc, "policies/returns.md") },
            new[] { "Hi! I think my order might be defective — what are my options?" },
            "The only relevant policy is the 17-day return window for a full refund. A helpful reply acknowledges the concern, explains the return option, and is clear and courteous without inventing a warranty or repair process that isn't in the ground truth.",
            "Score 5 if the reply is clear, courteous, and helpful: it acknowledges the defect concern and points to the available return option (17-day window) without fabricating a warranty/repair policy that doesn't exist in the ground truth. Score low if it is curt, unhelpful, or invents policies."),
    };
}
