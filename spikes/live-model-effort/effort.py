import subprocess, json, time, threading
p = subprocess.Popen(["claude","-p","--input-format","stream-json","--output-format","stream-json","--verbose","--model","haiku","--effort","low"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd="/tmp/dt-model-spike")
def send(o): p.stdin.write(json.dumps(o)+"\n"); p.stdin.flush()
out=[]
def rd():
    for l in p.stdout: out.append(l)
threading.Thread(target=rd,daemon=True).start()
send({"type":"control_request","request_id":"r1","request":{"subtype":"initialize"}})
time.sleep(4)
send({"type":"control_request","request_id":"g1","request":{"subtype":"get_settings"}})
time.sleep(1)
send({"type":"control_request","request_id":"a1","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"high"}}})
time.sleep(1)
send({"type":"control_request","request_id":"g2","request":{"subtype":"get_settings"}})
time.sleep(1)
send({"type":"control_request","request_id":"a2","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"bogus"}}})
time.sleep(1)
send({"type":"control_request","request_id":"g3","request":{"subtype":"get_settings"}})
time.sleep(1)
send({"type":"control_request","request_id":"a3","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"max"}}})
send({"type":"control_request","request_id":"s1","request":{"subtype":"set_model","model":"claude-haiku-4-5-20251001"}})
time.sleep(1)
send({"type":"control_request","request_id":"g4","request":{"subtype":"get_settings"}})
time.sleep(2)
p.stdin.close(); p.wait(timeout=20)
for l in out:
    o=json.loads(l)
    if o.get("type")=="control_response":
        r=o["response"]; rid=r.get("request_id")
        if rid=="r1": continue
        if rid.startswith("g"):
            body=r.get("response",{})
            def find(d,path=""):
                if isinstance(d,dict):
                    for k,v in d.items():
                        if "ffort" in k or k=="model" or "Model" in k: print(rid,path+"/"+k,"=",json.dumps(v)[:200])
                        else: find(v,path+"/"+k)
            find(body)
            print(rid, "top keys:", list(body.keys()))
        else:
            print(json.dumps(o)[:400])
