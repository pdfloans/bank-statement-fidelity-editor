import sys
import os
import pytest

# Add parent directory to sys.path to import modules
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..')))

from font_replicator import _char_class, _decompose_to_components, _infer_weight_width

def test_char_class():
    assert _char_class('5') == 'digit'
    assert _char_class('A') == 'upper'
    assert _char_class('z') == 'lower'
    assert _char_class('-') == 'other'
    assert _char_class('.') == 'other'

def test_decompose_to_components():
    assert _decompose_to_components('e') == ['e']
    # 'é' is \u00e9, decomps to 'e' and '\u0301'
    decomp = _decompose_to_components('é')
    assert len(decomp) == 2
    assert decomp[0] == 'e'
    assert decomp[1] == '\u0301'

def test_infer_weight_width():
    assert _infer_weight_width("Helvetica-Bold") == (700, 5)
    assert _infer_weight_width("Arial-Italic") == (400, 5)
    assert _infer_weight_width("Roboto-Light") == (300, 5)
    assert _infer_weight_width("OpenSans-CondensedBold") == (700, 3)
    assert _infer_weight_width("SomeFont-BlackExtended") == (900, 7)
