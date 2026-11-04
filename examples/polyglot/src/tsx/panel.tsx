// @openshell egress api.anthropic.com:443 reason="messages API" allow="POST:/v1/messages"
// @openshell provider ANTHROPIC_API_KEY profile=anthropic name=anthropic
export async function Panel(props: { prompt: string }) {
  const res = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: { "x-api-key": process.env.ANTHROPIC_API_KEY ?? "" },
    body: JSON.stringify({
      model: "claude",
      messages: [{ role: "user", content: props.prompt }],
    }),
  });
  const text = await res.text();
  return <pre>{text}</pre>;
}
