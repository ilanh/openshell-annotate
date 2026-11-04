// @openshell egress api.openai.com:443 reason="chat completions" allow="POST:/v1/chat/completions"
// @openshell provider OPENAI_API_KEY profile=openai name=openai-chat
export async function complete(prompt: string): Promise<string> {
  const res = await fetch("https://api.openai.com/v1/chat/completions", {
    method: "POST",
    headers: { Authorization: `Bearer ${process.env.OPENAI_API_KEY}` },
    body: JSON.stringify({ model: "gpt-4o-mini", messages: [{ role: "user", content: prompt }] }),
  });
  return res.text();
}
