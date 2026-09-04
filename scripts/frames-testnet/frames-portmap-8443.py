import re, urllib.request
DESC="http://192.168.1.1:49652/49652gatedesc.xml"
import xml.etree.ElementTree as ET
root=ET.fromstring(urllib.request.urlopen(DESC,timeout=10).read())
ns={"d":"urn:schemas-upnp-org:device-1-0"}
base=re.match(r"(http://[^/]+)",DESC).group(1)
for svc in root.iter("{urn:schemas-upnp-org:device-1-0}service"):
    st=svc.findtext("d:serviceType","",ns)
    if "WANIPConnection" in st:
        ctrl=svc.findtext("d:controlURL","",ns)
        if not ctrl.startswith("http"): ctrl=base+("" if ctrl.startswith("/") else "/")+ctrl
        break
body=("<NewRemoteHost></NewRemoteHost><NewExternalPort>8443</NewExternalPort>"
      "<NewProtocol>TCP</NewProtocol><NewInternalPort>8443</NewInternalPort>"
      "<NewInternalClient>192.168.1.3</NewInternalClient><NewEnabled>1</NewEnabled>"
      "<NewPortMappingDescription>frames-checkpoint-8443</NewPortMappingDescription>"
      "<NewLeaseDuration>0</NewLeaseDuration>")
env=('<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" '
     's:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body>'
     f'<u:AddPortMapping xmlns:u="{st}">{body}</u:AddPortMapping></s:Body></s:Envelope>').encode()
req=urllib.request.Request(ctrl,data=env,headers={"Content-Type":'text/xml; charset="utf-8"',"SOAPAction":f'"{st}#AddPortMapping"'})
urllib.request.urlopen(req,timeout=15).read()
print("mapped TCP 8443 -> 192.168.1.3:8443")
