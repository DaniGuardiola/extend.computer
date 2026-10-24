import json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import Mock,patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
import network_route
from client import Client,RoutedHTTPSHandler


class NetworkRouteTests(unittest.TestCase):
    def test_address_validation_accepts_cable_hosts_and_rejects_nonhosts(self):
        for value in ['169.254.173.134','169.254.226.32','10.0.100.23','192.168.1.23']:
            self.assertEqual(network_route.private_ipv4(value),value)
        for value in ['0.0.0.0','127.0.0.1','224.0.0.1','255.255.255.255','8.8.8.8','::1','fe80::1']:
            with self.subTest(value=value),self.assertRaises(ValueError):
                network_route.private_ipv4(value)

    def test_mac_socket_binds_source_before_interface_and_connect(self):
        sock=Mock();events=[]
        sock.bind.side_effect=lambda address:events.append(('bind',address))
        sock.setsockopt.side_effect=lambda *args:events.append(('scope',args))
        sock.connect.side_effect=lambda address:events.append(('connect',address))
        with patch.object(network_route.sys,'platform','darwin'),patch.object(network_route.socket,'socket',return_value=sock),patch.object(network_route.socket,'if_nametoindex',return_value=7):
            actual=network_route.routed_connection(('169.254.173.134',8877),10,('169.254.226.32',0),'en-test')
        self.assertIs(actual,sock)
        self.assertEqual([event[0] for event in events],['bind','scope','connect'])
        self.assertEqual(events[0][1],('169.254.226.32',0))
        self.assertEqual(events[1][1],(network_route.socket.IPPROTO_IP,25,7))
        self.assertEqual(events[2][1],('169.254.173.134',8877))

    def test_missing_interface_closes_socket_without_connecting(self):
        sock=Mock()
        with patch.object(network_route.sys,'platform','darwin'),patch.object(network_route.socket,'socket',return_value=sock),patch.object(network_route.socket,'if_nametoindex',side_effect=OSError('No such interface')):
            with self.assertRaises(OSError):
                network_route.routed_connection(('169.254.173.134',8877),10,('169.254.226.32',0),'missing')
        sock.connect.assert_not_called();sock.close.assert_called_once()

    def test_scoped_https_uses_pinned_context_and_ignores_proxy_environment(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'pairing.json'
            path.write_text(json.dumps({'url':'https://169.254.173.134:8877','token':'test-token','certificate':'test-certificate','routing':{'source_ipv4':'169.254.226.32','interface':'en-test'}}))
            context=Mock();opener=Mock()
            with patch('client.ssl.create_default_context',return_value=context) as create,patch('client.urllib.request.build_opener',return_value=opener) as build:
                client=Client(path)
            create.assert_called_once_with(cadata='test-certificate')
            proxy,handler=build.call_args.args
            self.assertEqual(proxy.proxies,{})
            self.assertIsInstance(handler,RoutedHTTPSHandler)
            request=Mock()
            with patch.object(handler,'do_open',return_value='opened') as do_open:
                self.assertEqual(handler.https_open(request),'opened')
            factory=do_open.call_args.args[0]
            connection=factory('169.254.173.134:8877',timeout=60,context=context)
            self.assertIs(connection._context,context)
            with patch('client.routed_connection',return_value='connected') as connect:
                self.assertEqual(connection._create_connection(('169.254.173.134',8877),60,connection.source_address),'connected')
            connect.assert_called_once_with(('169.254.173.134',8877),60,('169.254.226.32',0),'en-test')

    def test_existing_unscoped_pairing_preserves_default_request_path(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'pairing.json';path.write_text(json.dumps({'url':'https://10.0.100.23:8877','token':'test-token','certificate':'test-certificate'}))
            with patch('client.ssl.create_default_context'),patch('client.urllib.request.build_opener') as build:
                self.assertIsNone(Client(path).opener)
            build.assert_not_called()


if __name__=='__main__':unittest.main()
