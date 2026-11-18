import io.firebus.Address;
import io.firebus.Directory;
import io.firebus.Message;
import io.firebus.Payload;
import io.firebus.data.DataList;
import io.firebus.data.DataMap;
import io.firebus.information.NodeInformation;
import io.firebus.information.ServiceInformation;
import io.firebus.information.StreamInformation;

import javax.crypto.SecretKeyFactory;
import javax.crypto.spec.IvParameterSpec;
import javax.crypto.spec.PBEKeySpec;
import javax.crypto.Cipher;
import javax.crypto.spec.SecretKeySpec;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.spec.KeySpec;
import java.util.Arrays;
import java.util.HashMap;

public class VectorGen
{
	public static void main(String[] args) throws Exception
	{
		Path out = Paths.get(args[0]);
		Files.createDirectories(out);

		// ---------------- Crypto vectors ----------------
		byte[] salt = {(byte)98, 17, (byte)213, 33, (byte)198, (byte)234, 38, 87, (byte)251, (byte)194, 67, 71, 9, 54, (byte)201, 12};
		SecretKeyFactory f = SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256");

		KeySpec spec = new PBEKeySpec("firebuspassword0".toCharArray(), salt, 65536, 256);
		byte[] key = f.generateSecret(spec).getEncoded();
		write(out, "java_pbkdf2_key_default.hex", key);

		KeySpec spec2 = new PBEKeySpec("anotherpassword123".toCharArray(), salt, 65536, 256);
		byte[] key2 = f.generateSecret(spec2).getEncoded();
		write(out, "java_pbkdf2_key_alt.hex", key2);

		byte[] plaintext = "Firebus CFB8 golden vector 0123456789 ABCDEF!".getBytes(StandardCharsets.UTF_8);
		write(out, "java_cfb8_plaintext.hex", plaintext);

		byte[] iv1 = new byte[16];
		for (int i = 0; i < 16; i++) iv1[i] = (byte) i;
		write(out, "java_cfb8_iv1.hex", iv1);

		byte[] iv2 = new byte[16];
		for (int i = 0; i < 16; i++) iv2[i] = (byte) (0xA0 - i);
		write(out, "java_cfb8_iv2.hex", iv2);

		byte[] ct1 = cfb8(key, iv1, plaintext, true);
		if (!Arrays.equals(plaintext, cfb8(key, iv1, ct1, false)))
			throw new RuntimeException("CFB8 round trip failed");
		write(out, "java_cfb8_cipher1.hex", ct1);
		write(out, "java_cfb8_cipher2.hex", cfb8(key, iv2, plaintext, true));
		write(out, "java_cfb8_cipher3.hex", cfb8(key2, iv1, plaintext, true));

		// ---------------- Payload fixtures ----------------
		Payload pBytes = new Payload(null, "raw-bytes-0123456789".getBytes(StandardCharsets.UTF_8));
		write(out, "java_payload_bytes.hex", pBytes.serialise());

		HashMap<String, String> m1 = new HashMap<String, String>();
		m1.put("token", "bebop");
		Payload pStr = new Payload("some string payload");
		pStr.metadata = m1;
		write(out, "java_payload_string_meta1.hex", pStr.serialise());

		HashMap<String, String> m2 = new HashMap<String, String>();
		m2.put("token", "bebop");
		m2.put("other", "value2");
		Payload pMap = new Payload(new DataMap("key", "value"));
		pMap.metadata = m2;
		write(out, "java_payload_datamap.hex", pMap.serialise());

		DataList dl = new DataList();
		dl.add(1);
		dl.add("two");
		dl.add(3.5);
		dl.add(true);
		write(out, "java_payload_datalist.hex", new Payload(dl).serialise());

		write(out, "java_payload_empty.hex", new Payload().serialise());

		// ---------------- Message fixtures ----------------
		Message reqMsg = new Message(222, 111, Message.MSGTYPE_REQUESTSERVICE, "service01", pMap);
		reqMsg.setCorrelation(42, 7);
		reqMsg.setRepeatsLeft(3);
		write(out, "java_msg_request.hex", reqMsg.serialise());

		Directory dir = new Directory();
		NodeInformation ni = dir.getOrCreateNodeInformation(12345);
		ni.addAddress(new Address("10.1.2.3", 1990));
		ni.addAddress(new Address("10.1.2.4", 1991));
		ni.addRepeater(54321);
		NodeInformation ni2 = dir.getOrCreateNodeInformation(54321);
		ni2.addAddress(new Address("192.168.0.9", 2000));
		String snapshot = "777,a,10.0.0.1,1990\r\n"
				+ "777,f,s,serviceOne\r\n"
				+ "777,f,t,streamOne\r\n"
				+ "777,f,c,consumerOne\r\n"
				+ dir.getDirectoryStateString(777);
		Files.write(out.resolve("java_directory_snapshot.txt"), snapshot.getBytes(StandardCharsets.ISO_8859_1));

		Message infoMsg = new Message(111, 222, Message.MSGTYPE_NODEINFORMATION, null,
				new Payload(null, snapshot.getBytes(StandardCharsets.ISO_8859_1)));
		infoMsg.setCorrelation(42, 0);
		write(out, "java_msg_nodeinfo.hex", infoMsg.serialise());

		// ---------------- Function information fixtures ----------------
		byte[] siBytes = funcInfo('s', "application/json", "request-contract-s", "text/html", "response-contract-s");
		ServiceInformation si = new ServiceInformation(new NodeInformation(777), "serviceOne");
		si.deserialise(siBytes);
		if (!si.getRequestMimeType().equals("application/json") || !si.getResponseContract().equals("response-contract-s"))
			throw new RuntimeException("ServiceInformation deserialise mismatch");
		write(out, "java_funcinfo_service.hex", siBytes);

		byte[] tiBytes = funcInfo('t', "application/octet-stream", null, null, "response-contract-t");
		StreamInformation sti = new StreamInformation(new NodeInformation(777), "streamOne");
		sti.deserialise(tiBytes);
		if (!sti.getRequestMimeType().equals("application/octet-stream") || sti.getResponseContract() == null
				|| !sti.getResponseContract().equals("response-contract-t"))
			throw new RuntimeException("StreamInformation deserialise mismatch");
		write(out, "java_funcinfo_stream.hex", tiBytes);

		byte[] tiEmpty = funcInfo('t', null, null, null, null);
		if (tiEmpty.length != 11)
			throw new RuntimeException("empty funcinfo wrong length");
		write(out, "java_funcinfo_stream_empty.hex", tiEmpty);

		int crc = 0;
		for (byte b : reqMsg.serialise())
			crc = (crc ^ b) & 0xFF;
		write(out, "java_framed_request.hex", frame(reqMsg.serialise()));
		write(out, "java_framed_nodeinfo.hex", frame(infoMsg.serialise()));

		// ---------------- Framing fixture: CRC boundary cases ----------------
		// a synthetic payload whose XOR-CRC byte is 0x7E (framing edge case)
		byte[] data = new byte[8];
		for (int i = 0; i < 8; i++) data[i] = 0x01;
		int c = 0;
		for (byte b : data) c = (c ^ b) & 0xFF; // 0x01
		int target = 0x7E;
		data[0] = (byte) (data[0] ^ (c ^ target)); // makes crc == 0x7E
		write(out, "java_framed_crc7e.hex", frame(data));

		System.out.println("vectors written to " + out.toAbsolutePath());
	}

	private static byte[] cfb8(byte[] keyBytes, byte[] iv, byte[] data, boolean enc) throws Exception
	{
		Cipher c = Cipher.getInstance("AES/CFB8/NoPadding");
		SecretKeySpec k = new SecretKeySpec(keyBytes, "AES");
		if (enc)
			c.init(Cipher.ENCRYPT_MODE, k, new IvParameterSpec(iv));
		else
			c.init(Cipher.DECRYPT_MODE, k, new IvParameterSpec(iv));
		return c.doFinal(data);
	}

	private static byte[] funcInfo(char type, String mime, String contract, String mime2, String contract2)
	{
		ByteArrayBuilder bb = new ByteArrayBuilder();
		bb.put((byte) type);
		putStr8(bb, mime);
		putStr32(bb, contract);
		putStr8(bb, mime2);
		putStr32(bb, contract2);
		return bb.bytes();
	}

	private static void putStr8(ByteArrayBuilder bb, String s)
	{
		if (s != null)
		{
			byte[] b = s.getBytes(StandardCharsets.US_ASCII);
			bb.put((byte) b.length);
			bb.putAll(b);
		}
		else
			bb.put((byte) 0);
	}

	private static void putStr32(ByteArrayBuilder bb, String s)
	{
		if (s != null)
		{
			byte[] b = s.getBytes(StandardCharsets.US_ASCII);
			bb.putInt(b.length);
			bb.putAll(b);
		}
		else
			bb.putInt(0);
	}

	private static byte[] frame(byte[] msgData)
	{
		int crc = 0;
		for (int i = 0; i < msgData.length; i++)
			crc = (crc ^ msgData[i]) & 0x00FF;
		int packetSize = msgData.length + 6;
		packetSize += 16 - (packetSize % 16);
		byte[] bytes = new byte[packetSize];
		bytes[0] = 0x7E;
		bytes[1] = (byte) (msgData.length & 0x000000FF);
		bytes[2] = (byte) ((msgData.length >> 8) & 0x000000FF);
		bytes[3] = (byte) ((msgData.length >> 16) & 0x000000FF);
		bytes[4] = (byte) ((msgData.length >> 24) & 0x000000FF);
		for (int i = 0; i < msgData.length; i++)
			bytes[5 + i] = msgData[i];
		bytes[msgData.length + 5] = (byte) crc;
		return bytes;
	}

	private static void write(Path dir, String name, byte[] data) throws Exception
	{
		StringBuilder sb = new StringBuilder();
		for (byte b : data)
			sb.append(String.format("%02X", b));
		Files.write(dir.resolve(name), sb.toString().getBytes(StandardCharsets.US_ASCII));
	}

	private static class ByteArrayBuilder
	{
		private java.io.ByteArrayOutputStream bos = new java.io.ByteArrayOutputStream();

		void put(byte b)
		{
			bos.write(b);
		}

		void putAll(byte[] b)
		{
			try
			{
				bos.write(b);
			}
			catch (Exception e)
			{
			}
		}

		void putInt(int v)
		{
			// java.nio.ByteBuffer default is big-endian
			put((byte) ((v >> 24) & 0xFF));
			put((byte) ((v >> 16) & 0xFF));
			put((byte) ((v >> 8) & 0xFF));
			put((byte) (v & 0xFF));
		}

		byte[] bytes()
		{
			return bos.toByteArray();
		}
	}
}
