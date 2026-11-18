import io.firebus.Message;
import io.firebus.Payload;
import io.firebus.data.DataMap;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * Verifies the RUST -> JAVA interop direction: deserializes the byte sequences produced by
 * the Rust implementation (tests/golden/rust_*.hex, see src/bin/gen_golden.rs) with the Java
 * classes and asserts the logical content. Runs as part of tools/golden/verify.sh.
 */
public class VectorVerify
{
	public static void main(String[] args) throws Exception
	{
		Path dir = Paths.get(args[0]);

		payloadBytes(dir);
		payloadStringMeta(dir);
		payloadDataMap(dir);
		messageRequest(dir);
		messageNodeInfo(dir);

		// Logical round-trip: parse a Rust byte sequence, re-serialize it with Java,
		// parse the Java re-serialization and confirm identical logical content.
		logicalRoundTrip(dir, "rust_payload_bytes.hex", "payload_bytes");
		logicalRoundTrip(dir, "rust_payload_string_meta1.hex", "payload_string_meta1");
		logicalRoundTrip(dir, "rust_payload_datamap.hex", "payload_datamap");
		logicalRoundTrip(dir, "rust_msg_request.hex", "msg_request");

		System.out.println("VectorVerify OK: all Rust fixtures parsed and verified by Java");
	}

	private static void logicalRoundTrip(Path dir, String name, String what) throws Exception
	{
		byte[] orig = read(dir, name);
		Message m = (name.startsWith("rust_msg")) ? Message.deserialise(orig) : null;
		Payload a = (m != null) ? m.getPayload() : Payload.deserialise(orig);
		byte[] re = (m != null) ? m.serialise() : a.serialise();
		Payload b = (m != null) ? Message.deserialise(re).getPayload() : Payload.deserialise(re);
		check(a.getDataType() == b.getDataType(), what + " round-trip type");
		check(metaStr(a).equals(metaStr(b)), what + " round-trip metadata");
		check(a.getBytes() != null && java.util.Arrays.equals(a.getBytes(), b.getBytes()),
				what + " round-trip bytes (re-parse stable)");
	}

	private static String metaStr(Payload p)
	{
		StringBuilder sb = new StringBuilder();
		java.util.TreeMap<String, String> sorted = new java.util.TreeMap<String, String>(p.metadata);
		for (java.util.Map.Entry<String, String> e : sorted.entrySet())
			sb.append(e.getKey()).append('=').append(e.getValue()).append(';');
		return sb.toString();
	}

	private static void payloadBytes(Path dir) throws Exception
	{
		Payload p = Payload.deserialise(read(dir, "rust_payload_bytes.hex"));
		check(p.getDataType() == Payload.TYPE_BYTES, "payload_bytes type");
		check("raw-bytes-0123456789".equals(new String(p.getBytes(), StandardCharsets.UTF_8)), "payload_bytes content");
		check(p.metadata.isEmpty(), "payload_bytes metadata empty");
	}

	private static void payloadStringMeta(Path dir) throws Exception
	{
		Payload p = Payload.deserialise(read(dir, "rust_payload_string_meta1.hex"));
		check(p.getDataType() == Payload.TYPE_STRING, "payload_string type");
		check("some string payload".equals(p.getString()), "payload_string content");
		check("bebop".equals(p.metadata.get("token")), "payload_string metadata");
	}

	private static void payloadDataMap(Path dir) throws Exception
	{
		Payload p = Payload.deserialise(read(dir, "rust_payload_datamap.hex"));
		check(p.getDataType() == Payload.TYPE_DATAMAP, "payload_datamap type");
		DataMap m = p.getDataMap();
		check(m.getString("key") != null && m.getString("key").equals("value"), "payload_datamap content");
		check("bebop".equals(p.metadata.get("token")), "payload_datamap metadata token");
		check("value2".equals(p.metadata.get("other")), "payload_datamap metadata other");
	}

	private static void messageRequest(Path dir) throws Exception
	{
		Message m = Message.deserialise(read(dir, "rust_msg_request.hex"));
		check(m.getDestinationId() == 222, "msg_request dest");
		check(m.getOriginatorId() == 111, "msg_request origin");
		check(m.getType() == Message.MSGTYPE_REQUESTSERVICE, "msg_request type");
		check("service01".equals(m.getSubject()), "msg_request subject");
		check(m.getCorrelation() == 42, "msg_request correlation");
		check(m.getCorrelationSequence() == 7, "msg_request corr seq");
		check(m.getRepeatsLeft() == 3, "msg_request repeats left");
		Payload p = m.getPayload();
		check(p.metadata.size() == 2, "msg_request payload metadata size");
		check(p.getDataType() == Payload.TYPE_DATAMAP, "msg_request payload present");
		DataMap dm = p.getDataMap();
		check("value".equals(dm.getString("key")), "msg_request payload map");
	}

	private static void messageNodeInfo(Path dir) throws Exception
	{
		Message m = Message.deserialise(read(dir, "rust_msg_nodeinfo.hex"));
		check(m.getDestinationId() == 111, "msg_nodeinfo dest");
		check(m.getOriginatorId() == 222, "msg_nodeinfo origin");
		check(m.getType() == Message.MSGTYPE_NODEINFORMATION, "msg_nodeinfo type");
		check(m.getSubject() == null || m.getSubject().isEmpty(), "msg_nodeinfo subject");
		check(m.getCorrelation() == 42, "msg_nodeinfo correlation");
		String text = new String(m.getPayload().getBytes(), StandardCharsets.ISO_8859_1);
		check(text.contains("777,f,s,serviceOne"), "msg_nodeinfo snapshot functions");
		check(text.contains("777,d,12345,a,10.1.2.3,1990"), "msg_nodeinfo snapshot directory");
	}

	private static byte[] read(Path dir, String name) throws IOException
	{
		String hex = new String(Files.readAllBytes(dir.resolve(name)), StandardCharsets.US_ASCII).trim();
		byte[] out = new byte[hex.length() / 2];
		for (int i = 0; i < out.length; i++)
			out[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
		return out;
	}

	private static void check(boolean ok, String what)
	{
		if (!ok)
		{
			throw new RuntimeException("FAILED: " + what);
		}
		System.out.println("ok: " + what);
	}
}
