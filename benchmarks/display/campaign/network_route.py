"""Optional socket-local routing for benchmark links on macOS."""
import ipaddress
import socket
import sys


def private_ipv4(value):
    ip = ipaddress.ip_address(value)
    if (ip.version != 4 or not (ip.is_private or ip.is_link_local)
            or ip.is_loopback or ip.is_unspecified or ip.is_multicast or ip.is_reserved):
        raise ValueError('Use a private or link-local IPv4 host address')
    return str(ip)


def bind_interface(sock, interface):
    if sys.platform != 'darwin':
        raise ValueError('Interface-scoped benchmark routing requires macOS')
    if not isinstance(interface, str) or not interface:
        raise ValueError('A network interface name is required')
    index = socket.if_nametoindex(interface)
    sock.setsockopt(socket.IPPROTO_IP, getattr(socket, 'IP_BOUND_IF', 25), index)


def routed_connection(address, timeout, source_address, interface):
    destination = private_ipv4(address[0])
    source = private_ipv4(source_address[0])
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        if timeout is not socket._GLOBAL_DEFAULT_TIMEOUT:
            sock.settimeout(timeout)
        # macOS VPN routing can replace a scope applied before the source bind.
        sock.bind((source, source_address[1]))
        bind_interface(sock, interface)
        sock.connect((destination, address[1]))
        return sock
    except BaseException:
        sock.close()
        raise
