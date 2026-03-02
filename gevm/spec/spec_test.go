package spec

import "testing"

func TestForkIDOrdering(t *testing.T) {
	if Frontier >= Homestead {
		t.Error("Frontier should be before Homestead")
	}
	if London >= Shanghai {
		t.Error("London should be before Shanghai")
	}
	if Cancun >= Prague {
		t.Error("Cancun should be before Prague")
	}
	if Prague >= Osaka {
		t.Error("Prague should be before Osaka")
	}
}

func TestIsEnabledIn(t *testing.T) {
	// Shanghai is enabled in Cancun (Cancun >= Shanghai)
	if !Cancun.IsEnabledIn(Shanghai) {
		t.Error("Shanghai should be enabled in Cancun")
	}
	// Cancun is NOT enabled in Shanghai (Shanghai < Cancun)
	if Shanghai.IsEnabledIn(Cancun) {
		t.Error("Cancun should NOT be enabled in Shanghai")
	}
	// A fork is enabled in itself
	if !London.IsEnabledIn(London) {
		t.Error("London should be enabled in itself")
	}
}

func TestForkIDString(t *testing.T) {
	if Frontier.String() != "Frontier" {
		t.Errorf("Frontier string: got %s", Frontier.String())
	}
	if Prague.String() != "Prague" {
		t.Errorf("Prague string: got %s", Prague.String())
	}
	if Osaka.String() != "Osaka" {
		t.Errorf("Osaka string: got %s", Osaka.String())
	}
}

func TestForkIDFromString(t *testing.T) {
	s, err := ForkIDFromString("Prague")
	if err != nil || s != Prague {
		t.Errorf("ForkIDFromString(Prague): got %v, err %v", s, err)
	}

	s, err = ForkIDFromString("Latest")
	if err != nil || s != LatestForkID {
		t.Errorf("ForkIDFromString(Latest): got %v, err %v", s, err)
	}

	_, err = ForkIDFromString("Nonexistent")
	if err == nil {
		t.Error("ForkIDFromString(Nonexistent) should return error")
	}
}

func TestTryFromU8(t *testing.T) {
	s, err := ForkIDFromByte(0)
	if err != nil || s != Frontier {
		t.Errorf("ForkIDFromByte(0): got %v, err %v", s, err)
	}

	_, err = ForkIDFromByte(255)
	if err == nil {
		t.Error("ForkIDFromByte(255) should return error")
	}
}

func TestLatestForkID(t *testing.T) {
	if LatestForkID != Osaka {
		t.Errorf("LatestForkID should be Osaka, got %v", LatestForkID)
	}
}
